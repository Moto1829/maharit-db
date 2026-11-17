//! 実際に TCP で接続して確かめる結合テスト（ユーザー管理・ACL・トランザクション・並行書き込み・メトリクスなど）。

use super::*;

#[tokio::test]
async fn integration_ping_returns_pong() {
    let addr = start_test_server().await;
    let mut stream = TcpStream::connect(addr).await.unwrap();

    let resp = send_recv(&mut stream, &Request::Ping).await;
    assert!(
        matches!(resp, Response::Pong),
        "expected Pong, got {:?}",
        resp
    );
}

#[tokio::test]
async fn integration_user_management_over_tcp() {
    let addr = start_test_server().await;
    let mut stream = TcpStream::connect(addr).await.unwrap();

    let q = |s: &str| Request::Query {
        query: s.to_string(),
        params: HashMap::new(),
        tx_id: None,
        session_token: None,
    };

    // CREATE USER (test's `SET PASSWORD .. SET ROLE '..'` grammar).
    let resp = send_recv(
        &mut stream,
        &q("CREATE USER alice SET PASSWORD 'pw1' SET ROLE 'reader'"),
    )
    .await;
    assert!(
        matches!(resp, Response::Result { .. }),
        "CREATE USER failed: {:?}",
        resp
    );

    // SHOW USERS must list admin + alice (no longer a placeholder).
    let resp = send_recv(&mut stream, &q("SHOW USERS")).await;
    let rows = match resp {
        Response::Result { rows } => rows,
        other => panic!("expected Result, got {:?}", other),
    };
    let joined = format!("{:?}", rows);
    assert!(
        joined.contains("alice"),
        "SHOW USERS missing alice: {}",
        joined
    );
    assert!(
        joined.contains("admin"),
        "SHOW USERS missing admin: {}",
        joined
    );

    // DROP USER of a nonexistent user must error (not silently succeed).
    let resp = send_recv(&mut stream, &q("DROP USER nobody")).await;
    assert!(
        matches!(resp, Response::Error { .. }),
        "DROP USER nonexistent should error, got {:?}",
        resp
    );

    // DROP the real user, then it is gone.
    let _ = send_recv(&mut stream, &q("DROP USER alice")).await;
    let resp = send_recv(&mut stream, &q("SHOW USERS")).await;
    if let Response::Result { rows } = resp {
        assert!(!format!("{:?}", rows).contains("alice"));
    } else {
        panic!("expected Result");
    }
}

/// ACL over TCP: hidden labels, write denial with rollback, admin bypass,
/// and GRANT / DENY / REVOKE / SHOW ACL restricted to admin (server/173).
#[tokio::test]
async fn integration_acl_over_tcp() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = TcpServer::new(ServerConfig {
        bind_address: addr.to_string(),
        require_auth: true,
        ..Default::default()
    })
    .with_auth(crate::auth::AuthManager::with_admin_password("adminpw"));
    tokio::spawn(async move {
        server.start_with_listener(listener).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    let mut stream = TcpStream::connect(addr).await.unwrap();

    async fn login(stream: &mut TcpStream, user: &str, pw: &str) -> String {
        let req = Request::Login {
            username: user.to_string(),
            password: pw.to_string(),
        };
        match send_recv(stream, &req).await {
            Response::LoggedIn { session_token, .. } => session_token,
            other => panic!("login failed: {:?}", other),
        }
    }
    async fn q(stream: &mut TcpStream, token: &str, query: &str) -> Response {
        let req = Request::Query {
            query: query.to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: Some(token.to_string()),
        };
        send_recv(stream, &req).await
    }
    async fn count(stream: &mut TcpStream, token: &str, query: &str) -> i64 {
        match q(stream, token, query).await {
            Response::Result { rows } => rows[0].values().next().unwrap().as_i64().unwrap(),
            other => panic!("{query}: {:?}", other),
        }
    }

    let admin = login(&mut stream, "admin", "adminpw").await;
    for stmt in [
        "CREATE (:Public {name: 'a'}), (:Secret {name: 's'}), (:Locked {name: 'l'})",
        "CREATE USER w SET PASSWORD 'wpw' ROLE writer",
        "DENY READ ON :Secret TO ROLE writer",
        "DENY WRITE ON :Locked TO USER w",
    ] {
        let resp = q(&mut stream, &admin, stmt).await;
        assert!(
            matches!(resp, Response::Result { .. }),
            "{stmt}: {:?}",
            resp
        );
    }
    // 存在しないユーザーへの規則はエラー
    let resp = q(&mut stream, &admin, "DENY READ ON :X TO USER nobody").await;
    assert!(matches!(resp, Response::Error { .. }), "{:?}", resp);

    let w = login(&mut stream, "w", "wpw").await;
    assert_eq!(
        count(&mut stream, &w, "MATCH (n) RETURN count(*) AS c").await,
        2
    );
    assert_eq!(
        count(&mut stream, &admin, "MATCH (n) RETURN count(*) AS c").await,
        3
    );

    // 書き込みの拒否は文全体を取り消す
    let resp = q(
        &mut stream,
        &w,
        "CREATE (:Public {name: 'b'}), (:Locked {name: 'm'})",
    )
    .await;
    match &resp {
        Response::Error { message } => assert!(message.contains("access denied"), "{message}"),
        other => panic!("expected access denied, got {:?}", other),
    }
    assert_eq!(
        count(&mut stream, &admin, "MATCH (n) RETURN count(*) AS c").await,
        3
    );
    let resp = q(&mut stream, &w, "CREATE (:Public {name: 'b'})").await;
    assert!(matches!(resp, Response::Result { .. }), "{:?}", resp);

    // ACL 文は admin だけ
    let resp = q(&mut stream, &w, "SHOW ACL").await;
    assert!(matches!(resp, Response::AuthError { .. }), "{:?}", resp);
    match q(&mut stream, &admin, "SHOW ACL").await {
        Response::Result { rows } => assert_eq!(rows.len(), 2, "{:?}", rows),
        other => panic!("{:?}", other),
    }

    let resp = q(
        &mut stream,
        &admin,
        "REVOKE READ ON :Secret FROM ROLE writer",
    )
    .await;
    assert!(matches!(resp, Response::Result { .. }), "{:?}", resp);
    assert_eq!(
        count(&mut stream, &w, "MATCH (n) RETURN count(*) AS c").await,
        4
    );
}

/// Concurrent writers must not overwrite each other's property-index
/// updates. Previously each query cloned the index and wrote the whole copy
/// back, so the last writer silently dropped the others' entries.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_writes_keep_property_index_consistent() {
    let addr = start_test_server().await;
    let mut setup = TcpStream::connect(addr).await.unwrap();
    query(&mut setup, "CREATE INDEX ON :Item(k)").await;

    const WRITERS: i64 = 8;
    const PER_WRITER: i64 = 40;
    let mut handles = Vec::new();
    for w in 0..WRITERS {
        handles.push(tokio::spawn(async move {
            let mut s = TcpStream::connect(addr).await.unwrap();
            for i in 0..PER_WRITER {
                let k = w * PER_WRITER + i;
                match query(&mut s, &format!("CREATE (:Item {{k: {k}}})")).await {
                    Response::Result { .. } => {}
                    other => panic!("CREATE failed: {:?}", other),
                }
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    // Every node must be reachable through the index-backed equality lookup.
    let mut missing = Vec::new();
    for k in 0..WRITERS * PER_WRITER {
        match query(
            &mut setup,
            &format!("MATCH (n:Item) WHERE n.k = {k} RETURN n.k"),
        )
        .await
        {
            Response::Result { rows } if rows.len() == 1 => {}
            _ => missing.push(k),
        }
    }
    assert!(
        missing.is_empty(),
        "index lost entries for k = {:?}",
        missing
    );
}

/// Concurrent CREATEs of the same UNIQUE value: exactly one may succeed.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_creates_respect_unique_constraint() {
    let addr = start_test_server().await;
    let mut setup = TcpStream::connect(addr).await.unwrap();
    query(
        &mut setup,
        "CREATE CONSTRAINT uniq_sku FOR (p:Product) REQUIRE p.sku IS UNIQUE",
    )
    .await;

    let mut handles = Vec::new();
    for _ in 0..16 {
        handles.push(tokio::spawn(async move {
            let mut s = TcpStream::connect(addr).await.unwrap();
            matches!(
                query(&mut s, "CREATE (:Product {sku: 'SKU-1'})").await,
                Response::Result { .. }
            )
        }));
    }
    let mut ok = 0;
    for h in handles {
        if h.await.unwrap() {
            ok += 1;
        }
    }
    assert_eq!(ok, 1, "exactly one CREATE must pass the UNIQUE check");
}

/// bug/160: a statement that fails midway leaves no changes behind, in or
/// outside a transaction, and the property index stays consistent.
#[tokio::test]
async fn failed_statement_is_undone() {
    let addr = start_test_server().await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    query(
        &mut s,
        "CREATE CONSTRAINT u_k FOR (n:U) REQUIRE n.k IS UNIQUE",
    )
    .await;
    query(&mut s, "CREATE INDEX ON :U(k)").await;
    query(&mut s, "CREATE (:U {k: 0, v: 'orig'})").await;

    let resp = query(&mut s, "UNWIND [1, 2, 2, 3] AS k CREATE (:U {k: k})").await;
    assert!(matches!(resp, Response::Error { .. }), "{resp:?}");
    assert_eq!(row_count(&mut s, "MATCH (n:U) RETURN n").await, 1);
    assert_eq!(row_count(&mut s, "MATCH (n:U {k: 1}) RETURN n").await, 0);

    // SET that fails on a later row: the earlier rows are restored too.
    query(&mut s, "CREATE (:U {k: 5, v: 'five'})").await;
    let resp = query(&mut s, "MATCH (n:U) SET n.v = 'x', n.k = 7").await;
    assert!(matches!(resp, Response::Error { .. }), "{resp:?}");
    assert_eq!(
        row_count(&mut s, "MATCH (n:U) WHERE n.v = 'x' RETURN n").await,
        0
    );
    assert_eq!(row_count(&mut s, "MATCH (n:U {k: 0}) RETURN n").await, 1);
    assert_eq!(row_count(&mut s, "MATCH (n:U {k: 7}) RETURN n").await, 0);

    // Inside a transaction: the failed statement is undone, earlier ones stay.
    let tx = begin(&mut s).await;
    tx_query(&mut s, tx, "CREATE (:U {k: 10})").await;
    let resp = tx_query(&mut s, tx, "UNWIND [11, 11] AS k CREATE (:U {k: k})").await;
    assert!(matches!(resp, Response::Error { .. }), "{resp:?}");
    assert_eq!(row_count(&mut s, "MATCH (n:U {k: 10}) RETURN n").await, 1);
    assert_eq!(row_count(&mut s, "MATCH (n:U {k: 11}) RETURN n").await, 0);
    rollback(&mut s, tx).await;
    assert_eq!(row_count(&mut s, "MATCH (n:U {k: 10}) RETURN n").await, 0);
    assert_eq!(row_count(&mut s, "MATCH (n:U) RETURN n").await, 2);
}

/// bug/159: a read-only transaction refuses writes, and a transaction left
/// open when its connection closes is rolled back.
#[tokio::test]
async fn read_only_transactions_and_disconnect_rollback() {
    let addr = start_test_server().await;
    let mut s = TcpStream::connect(addr).await.unwrap();

    let ro = match send_recv(
        &mut s,
        &Request::BeginTransaction {
            read_only: true,
            session_token: None,
        },
    )
    .await
    {
        Response::TransactionBegun { tx_id } => tx_id,
        other => panic!("begin failed: {other:?}"),
    };
    match tx_query(&mut s, ro, "CREATE (:T)").await {
        Response::Error { message } => assert!(message.contains("read-only"), "{message}"),
        other => panic!("write in a read-only transaction was accepted: {other:?}"),
    }
    assert!(matches!(
        tx_query(&mut s, ro, "MATCH (n) RETURN count(n) AS c").await,
        Response::Result { .. }
    ));
    assert_eq!(row_count(&mut s, "MATCH (n:T) RETURN n").await, 0);

    // Begin, write, and disconnect without committing.
    let mut other = TcpStream::connect(addr).await.unwrap();
    let tx = begin(&mut other).await;
    assert!(matches!(
        tx_query(&mut other, tx, "CREATE (:Left)").await,
        Response::Result { .. }
    ));
    assert_eq!(row_count(&mut s, "MATCH (n:Left) RETURN n").await, 1);
    drop(other);
    let mut gone = 1;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        gone = row_count(&mut s, "MATCH (n:Left) RETURN n").await;
        if gone == 0 {
            break;
        }
    }
    assert_eq!(
        gone, 0,
        "unfinished transaction was not rolled back on disconnect"
    );

    // A committed transaction stays.
    let mut third = TcpStream::connect(addr).await.unwrap();
    let tx = begin(&mut third).await;
    tx_query(&mut third, tx, "CREATE (:Kept)").await;
    let resp = send_recv(
        &mut third,
        &Request::Commit {
            tx_id: tx,
            session_token: None,
        },
    )
    .await;
    assert!(matches!(resp, Response::Committed { .. }), "{resp:?}");
    drop(third);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(row_count(&mut s, "MATCH (n:Kept) RETURN n").await, 1);
}

/// bug/156: the metrics served on /metrics reflect client activity.
#[tokio::test]
async fn metrics_record_queries_errors_connections_and_graph_size() {
    use crate::metrics::QueryType;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = TcpServer::new(ServerConfig {
        bind_address: addr.to_string(),
        ..Default::default()
    });
    let metrics = server.metrics();
    tokio::spawn(async move { server.start_with_listener(listener).await.unwrap() });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut s = TcpStream::connect(addr).await.unwrap();
    query(&mut s, "CREATE (:A)-[:R]->(:B)").await;
    query(&mut s, "MATCH (n) RETURN n").await;
    query(&mut s, "MATCH (n:A) SET n.x = 1").await;
    query(&mut s, "MATCH (n RETURN n").await;

    assert_eq!(metrics.query_count(QueryType::Create), 1);
    assert_eq!(metrics.query_count(QueryType::Match), 2);
    assert_eq!(metrics.query_count(QueryType::Set), 1);
    assert_eq!(metrics.error_count(), 1);
    assert_eq!(metrics.node_count(), 2);
    assert_eq!(metrics.edge_count(), 1);
    assert_eq!(metrics.current_connections(), 1);
    assert_eq!(metrics.total_connections(), 1);
    drop(s);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(metrics.current_connections(), 0);
}

/// bug/142: DETACH DELETE のサーバー経路（RecordingGraph）でも、ノードと
/// 一緒に消えたリレーションシップが deleted_edges に数えられる。
#[tokio::test]
async fn detach_delete_reports_removed_edges() {
    let addr = start_test_server().await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    query(&mut s, "CREATE (a:P {n: 1})-[:K]->(b:P {n: 2})").await;
    match query(&mut s, "MATCH (n:P {n: 2}) DETACH DELETE n").await {
        Response::Result { rows } => {
            assert_eq!(rows[0]["deleted_nodes"], serde_json::json!(1));
            assert_eq!(rows[0]["deleted_edges"], serde_json::json!(1));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// Changes made inside a transaction (SET on existing nodes, label changes,
/// CREATE, DETACH DELETE) run on the index-maintaining executor. ROLLBACK
/// must restore the graph *and* the property index; label changes and
/// DETACH-deleted edges used to be left un-restored (task117).
#[tokio::test]
async fn rollback_restores_graph_labels_edges_and_index() {
    let addr = start_test_server().await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    query(&mut s, "CREATE INDEX ON :Item(k)").await;
    query(&mut s, "CREATE (:Item {k: 1}), (:Item {k: 5})").await;
    query(
        &mut s,
        "MATCH (a:Item {k: 1}), (b:Item {k: 5}) CREATE (a)-[:R {w: 1}]->(b)",
    )
    .await;

    let tx = begin(&mut s).await;
    for q in [
        "MATCH (n:Item) WHERE n.k = 1 SET n.k = 2, n:Tmp",
        "CREATE (:Item {k: 9})",
        "MATCH (n:Item) WHERE n.k = 5 DETACH DELETE n",
    ] {
        let resp = tx_query(&mut s, tx, q).await;
        assert!(matches!(resp, Response::Result { .. }), "{q}: {resp:?}");
    }
    rollback(&mut s, tx).await;

    // Index-backed equality lookups must reflect the restored state.
    assert_eq!(
        row_count(&mut s, "MATCH (n:Item) WHERE n.k = 1 RETURN n").await,
        1
    );
    assert_eq!(
        row_count(&mut s, "MATCH (n:Item) WHERE n.k = 2 RETURN n").await,
        0
    );
    assert_eq!(
        row_count(&mut s, "MATCH (n:Item) WHERE n.k = 9 RETURN n").await,
        0
    );
    assert_eq!(
        row_count(&mut s, "MATCH (n:Item) WHERE n.k = 5 RETURN n").await,
        1
    );
    assert_eq!(row_count(&mut s, "MATCH (n:Tmp) RETURN n").await, 0);
    assert_eq!(
        row_count(&mut s, "MATCH (a:Item)-[r:R]->(b:Item) RETURN r.w").await,
        1,
        "DETACH-deleted edge must be restored"
    );
}

/// Writing with an unknown / finished transaction ID must be rejected
/// instead of applying changes that can never be rolled back.
#[tokio::test]
async fn write_with_inactive_tx_is_rejected() {
    let addr = start_test_server().await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    let tx = begin(&mut s).await;
    rollback(&mut s, tx).await;
    let resp = tx_query(&mut s, tx, "CREATE (:Ghost)").await;
    assert!(matches!(resp, Response::Error { .. }), "{resp:?}");
    assert_eq!(row_count(&mut s, "MATCH (n:Ghost) RETURN n").await, 0);
}

/// ROLLBACK on the leader must be replicated so followers end up with the
/// same data (task117).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rollback_is_replicated_to_followers() {
    use crate::replication::{
        FollowerReplicationManager, LeaderReplicationManager, NodeRole, ReplicationConfig,
    };

    let repl_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let repl_addr = repl_listener.local_addr().unwrap();
    let repl = Arc::new(LeaderReplicationManager::new(ReplicationConfig::default()));
    repl.start_with_listener(repl_listener).await.unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = TcpServer::new(ServerConfig {
        bind_address: addr.to_string(),
        read_timeout: Duration::from_secs(5),
        write_timeout: Duration::from_secs(5),
        ..Default::default()
    })
    .with_replication(Arc::clone(&repl));
    let leader_graph = server.graph_arc();
    tokio::spawn(async move {
        server.start_with_listener(listener).await.unwrap();
    });

    let follower_graph = Arc::new(ConcurrentGraph::new());
    let follower = FollowerReplicationManager::with_concurrent_graph(
        ReplicationConfig {
            role: NodeRole::Follower,
            node_id: "f-rollback".to_string(),
            replication_bind_address: "127.0.0.1:0".to_string(),
            leader_address: Some(repl_addr.to_string()),
            heartbeat_interval_secs: 1,
            heartbeat_timeout_secs: 5,
            shared_secret: None,
        },
        Arc::clone(&follower_graph),
    );
    let follower = Arc::new(follower);
    follower.start().await.unwrap();
    for _ in 0..50 {
        if repl.get_follower_count() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut s = TcpStream::connect(addr).await.unwrap();
    query(&mut s, "CREATE (:P {name: 'a'}), (:P {name: 'b'})").await;
    query(
        &mut s,
        "MATCH (a:P {name: 'a'}), (b:P {name: 'b'}) CREATE (a)-[:R]->(b)",
    )
    .await;

    let tx = begin(&mut s).await;
    tx_query(
        &mut s,
        tx,
        "MATCH (n:P {name: 'a'}) SET n.name = 'z', n:Tmp",
    )
    .await;
    tx_query(&mut s, tx, "CREATE (:P {name: 'c'})").await;
    tx_query(&mut s, tx, "MATCH (n:P {name: 'b'}) DETACH DELETE n").await;
    rollback(&mut s, tx).await;

    let snapshot = crate::replication_diff_test::fingerprint;

    let expected = snapshot(&leader_graph);
    assert_eq!(expected.0.len(), 2);
    assert_eq!(expected.1.len(), 1);
    let mut actual = snapshot(&follower_graph);
    for _ in 0..100 {
        if actual == expected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        actual = snapshot(&follower_graph);
    }
    assert_eq!(
        actual, expected,
        "follower diverged from leader after ROLLBACK"
    );
}

#[tokio::test]
async fn integration_property_index_persists_across_requests() {
    let addr = start_test_server().await;
    let mut stream = TcpStream::connect(addr).await.unwrap();

    // Seed a node and create an index in one request.
    let _ = send_recv(
        &mut stream,
        &Request::Query {
            query: "CREATE (:Widget {id: 1})".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    let _ = send_recv(
        &mut stream,
        &Request::Query {
            query: "CREATE INDEX ON :Widget(id)".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;

    // A subsequent, separate request must still see the index. Before the
    // property index was shared across requests, each Executor started with
    // an empty index and this returned zero rows.
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "SHOW INDEXES".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    let rows = match resp {
        Response::Result { rows } => rows,
        other => panic!("expected Result, got {:?}", other),
    };
    assert!(
        !rows.is_empty(),
        "property index must persist across requests (SHOW INDEXES returned no rows)"
    );
}

#[tokio::test]
async fn integration_repeated_query_uses_ast_cache_correctly() {
    let addr = start_test_server().await;
    let mut stream = TcpStream::connect(addr).await.unwrap();

    let read = Request::Query {
        query: "MATCH (n:Widget) RETURN n.id".to_string(),
        params: HashMap::new(),
        tx_id: None,
        session_token: None,
    };

    // First run against an empty graph.
    let resp = send_recv(&mut stream, &read).await;
    let count0 = match resp {
        Response::Result { rows } => rows.len(),
        other => panic!("expected Result, got {:?}", other),
    };
    assert_eq!(count0, 0);

    // Insert a Widget.
    let _ = send_recv(
        &mut stream,
        &Request::Query {
            query: "CREATE (n:Widget {id: 1})".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;

    // Re-run the identical read query. Its AST is served from the cache, but
    // the result must reflect the current graph (the cache stores the parsed
    // AST, never the result set).
    let resp = send_recv(&mut stream, &read).await;
    let count1 = match resp {
        Response::Result { rows } => rows.len(),
        other => panic!("expected Result, got {:?}", other),
    };
    assert_eq!(count1, 1, "cached AST must not cache stale results");
}

#[tokio::test]
async fn integration_create_and_match_node() {
    let addr = start_test_server().await;
    let mut stream = TcpStream::connect(addr).await.unwrap();

    // CREATE
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "CREATE (n:Person {name: 'Alice'}) RETURN n".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    assert!(
        matches!(resp, Response::Result { .. }),
        "CREATE failed: {:?}",
        resp
    );

    // MATCH
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "MATCH (n:Person {name: 'Alice'}) RETURN n.name".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    match resp {
        Response::Result { rows } => {
            assert!(!rows.is_empty(), "expected at least one row");
            // String values are returned as JSON-quoted strings (e.g. `"Alice"`)
            assert_eq!(
                rows[0].get("n.name").and_then(|v| v.as_str()),
                Some("Alice")
            );
        }
        other => panic!("expected Result, got {:?}", other),
    }
}

#[tokio::test]
async fn integration_property_types() {
    let addr = start_test_server().await;
    let mut stream = TcpStream::connect(addr).await.unwrap();

    // CREATE node with string, integer, float, boolean properties
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "CREATE (n:Item {s: 'hello', i: 42, f: 3.14, b: true}) RETURN n".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    assert!(
        matches!(resp, Response::Result { .. }),
        "CREATE failed: {:?}",
        resp
    );

    // MATCH each property individually
    // Server returns typed JSON values: String/Number/Boolean.
    for (col, expected) in [
        ("n.s", serde_json::json!("hello")),
        ("n.i", serde_json::json!(42)),
        ("n.b", serde_json::json!(true)),
    ] {
        let resp = send_recv(
            &mut stream,
            &Request::Query {
                query: format!("MATCH (n:Item) RETURN {}", col),
                params: HashMap::new(),
                tx_id: None,
                session_token: None,
            },
        )
        .await;
        match resp {
            Response::Result { rows } => {
                assert!(!rows.is_empty(), "no rows for {}", col);
                assert_eq!(rows[0].get(col), Some(&expected), "mismatch for {}", col);
            }
            other => panic!("expected Result for {}, got {:?}", col, other),
        }
    }

    // Float property — just check it parses as a number
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "MATCH (n:Item) RETURN n.f".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    match resp {
        Response::Result { rows } => {
            let val = rows[0]["n.f"].as_f64().expect("n.f should be a float");
            assert!((val - 3.14).abs() < 0.01);
        }
        other => panic!("expected Result for n.f, got {:?}", other),
    }
}

#[tokio::test]
async fn integration_edge_traversal() {
    let addr = start_test_server().await;
    let mut stream = TcpStream::connect(addr).await.unwrap();

    // Create two nodes and an edge between them
    for q in [
        "CREATE (n:A {name: 'src'})",
        "CREATE (n:B {name: 'dst'})",
        "MATCH (a:A {name: 'src'}), (b:B {name: 'dst'}) CREATE (a)-[:LINK]->(b)",
    ] {
        let resp = send_recv(
            &mut stream,
            &Request::Query {
                query: q.to_string(),
                params: HashMap::new(),
                tx_id: None,
                session_token: None,
            },
        )
        .await;
        assert!(
            matches!(resp, Response::Result { .. }),
            "setup query failed for '{}': {:?}",
            q,
            resp
        );
    }

    // Traverse: MATCH (a)-[:LINK]->(b) RETURN b.name
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "MATCH (a:A)-[:LINK]->(b:B) RETURN b.name".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    match resp {
        Response::Result { rows } => {
            assert!(!rows.is_empty(), "traversal returned no rows");
            assert_eq!(rows[0].get("b.name").and_then(|v| v.as_str()), Some("dst"));
        }
        other => panic!("expected Result, got {:?}", other),
    }
}

#[tokio::test]
async fn integration_syntax_error_returns_error_response() {
    let addr = start_test_server().await;
    let mut stream = TcpStream::connect(addr).await.unwrap();

    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "THIS IS NOT VALID CYPHER !!!".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    assert!(
        matches!(resp, Response::Error { .. }),
        "expected Error response, got {:?}",
        resp
    );
}

#[tokio::test]
async fn integration_immediate_disconnect_does_not_crash_server() {
    let addr = start_test_server().await;

    // Connect and drop immediately without sending anything
    let _stream = TcpStream::connect(addr).await.unwrap();
    drop(_stream);

    // Give the server time to handle the disconnect
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Server should still be alive — verify with a fresh ping
    let mut stream2 = TcpStream::connect(addr).await.unwrap();
    let resp = send_recv(&mut stream2, &Request::Ping).await;
    assert!(matches!(resp, Response::Pong));
}

#[tokio::test]
async fn integration_delete_node() {
    let addr = start_test_server().await;
    let mut stream = TcpStream::connect(addr).await.unwrap();

    // Create
    send_recv(
        &mut stream,
        &Request::Query {
            query: "CREATE (n:TmpNode {name: 'delete_me'})".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;

    // Delete
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "MATCH (n:TmpNode {name: 'delete_me'}) DETACH DELETE n".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    assert!(
        matches!(resp, Response::Result { .. }),
        "DELETE failed: {:?}",
        resp
    );

    // Verify gone
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "MATCH (n:TmpNode {name: 'delete_me'}) RETURN n.name".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    match resp {
        Response::Result { rows } => assert!(rows.is_empty(), "node should be deleted"),
        other => panic!("expected Result, got {:?}", other),
    }
}

// ── Concurrent connection tests ──────────────────────────────────────────

#[tokio::test]
async fn test_concurrent_writes() {
    // 10タスクが同時に異なるノードを作成し、全件が保存されることを確認
    let addr = start_test_server().await;

    let handles: Vec<_> = (0..10_u32)
        .map(|i| {
            tokio::spawn(async move {
                let mut stream = TcpStream::connect(addr).await.unwrap();
                let resp = send_recv(
                    &mut stream,
                    &Request::Query {
                        query: format!("CREATE (n:ConcWrite {{id: {i}}}) RETURN n"),
                        params: HashMap::new(),
                        tx_id: None,
                        session_token: None,
                    },
                )
                .await;
                assert!(
                    matches!(resp, Response::Result { .. }),
                    "concurrent CREATE #{i} failed: {resp:?}"
                );
            })
        })
        .collect();

    for h in handles {
        h.await.unwrap();
    }

    // 全件確認
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "MATCH (n:ConcWrite) RETURN n.id".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    match resp {
        Response::Result { rows } => {
            assert_eq!(
                rows.len(),
                10,
                "10件のConcWriteノードが存在するべき、実際: {}",
                rows.len()
            );
        }
        other => panic!("expected Result, got {other:?}"),
    }
}

#[tokio::test]
async fn test_concurrent_read_write_mix() {
    // 5スレッドが書き込み、5スレッドが読み取りを同時実行してもクラッシュしないことを確認
    let addr = start_test_server().await;

    // セットアップ: 読み取り用ノードを事前に作成
    let mut setup = TcpStream::connect(addr).await.unwrap();
    send_recv(
        &mut setup,
        &Request::Query {
            query: "CREATE (n:ReadTarget {val: 1})".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;

    let mut handles = Vec::new();

    // 5つの書き込みタスク
    for i in 0..5_u32 {
        handles.push(tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            let resp = send_recv(
                &mut stream,
                &Request::Query {
                    query: format!("CREATE (n:RWWrite {{id: {i}}}) RETURN n"),
                    params: HashMap::new(),
                    tx_id: None,
                    session_token: None,
                },
            )
            .await;
            assert!(
                matches!(resp, Response::Result { .. }),
                "write task #{i} failed: {resp:?}"
            );
        }));
    }

    // 5つの読み取りタスク
    for _ in 0..5_u32 {
        handles.push(tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            let resp = send_recv(
                &mut stream,
                &Request::Query {
                    query: "MATCH (n:ReadTarget) RETURN n.val".to_string(),
                    params: HashMap::new(),
                    tx_id: None,
                    session_token: None,
                },
            )
            .await;
            // エラーやパニックなく応答が返ること
            assert!(
                matches!(resp, Response::Result { .. }),
                "read task failed: {resp:?}"
            );
        }));
    }

    for h in handles {
        h.await.unwrap();
    }
}

#[tokio::test]
async fn test_max_connections_limit() {
    // max_connections=3 のサーバーに対して 6 クライアントが同時接続を試みても
    // サーバーがクラッシュせず、全クライアントが最終的に応答を受け取れることを確認
    let addr = start_test_server_with_config(3).await;

    let handles: Vec<_> = (0..6_u32)
        .map(|i| {
            tokio::spawn(async move {
                // 接続制限に引っかかる場合はサーバーがキューイングするため
                // 少し長めのタイムアウトで待つ
                let stream =
                    tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr)).await;

                // タイムアウトや接続拒否は許容（制限超過時の想定動作）
                if let Ok(Ok(mut s)) = stream {
                    // 接続できた場合は Ping を送って生存確認
                    let resp = send_recv(&mut s, &Request::Ping).await;
                    assert!(
                        matches!(resp, Response::Pong),
                        "client #{i} got unexpected response: {resp:?}"
                    );
                }
            })
        })
        .collect();

    for h in handles {
        h.await.unwrap();
    }

    // サーバーがまだ生きていることを確認
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let resp = send_recv(&mut stream, &Request::Ping).await;
    assert!(
        matches!(resp, Response::Pong),
        "server should still respond after connection burst"
    );
}

#[tokio::test]
async fn test_transaction_isolation() {
    // 2つのクライアントが同時にトランザクションを開始し、
    // 互いにコミット前の変更が干渉しないことを確認
    let addr = start_test_server().await;

    let mut client_a = TcpStream::connect(addr).await.unwrap();
    let mut client_b = TcpStream::connect(addr).await.unwrap();

    // クライアントA: トランザクション開始
    let resp_a = send_recv(
        &mut client_a,
        &Request::BeginTransaction {
            read_only: false,
            session_token: None,
        },
    )
    .await;
    let tx_a = match resp_a {
        Response::TransactionBegun { tx_id } => tx_id,
        other => panic!("client A expected TransactionBegun, got {other:?}"),
    };

    // クライアントB: トランザクション開始
    let resp_b = send_recv(
        &mut client_b,
        &Request::BeginTransaction {
            read_only: false,
            session_token: None,
        },
    )
    .await;
    let tx_b = match resp_b {
        Response::TransactionBegun { tx_id } => tx_id,
        other => panic!("client B expected TransactionBegun, got {other:?}"),
    };

    // クライアントA: トランザクション内でノードを作成
    let resp = send_recv(
        &mut client_a,
        &Request::Query {
            query: "CREATE (n:TxIsolate {owner: 'A'}) RETURN n".to_string(),
            params: HashMap::new(),
            tx_id: Some(tx_a),
            session_token: None,
        },
    )
    .await;
    assert!(
        matches!(resp, Response::Result { .. }),
        "tx A CREATE failed: {resp:?}"
    );

    // クライアントB: トランザクション内でノードを作成
    let resp = send_recv(
        &mut client_b,
        &Request::Query {
            query: "CREATE (n:TxIsolate {owner: 'B'}) RETURN n".to_string(),
            params: HashMap::new(),
            tx_id: Some(tx_b),
            session_token: None,
        },
    )
    .await;
    assert!(
        matches!(resp, Response::Result { .. }),
        "tx B CREATE failed: {resp:?}"
    );

    // クライアントA: コミット
    let resp = send_recv(
        &mut client_a,
        &Request::Commit {
            tx_id: tx_a,
            session_token: None,
        },
    )
    .await;
    assert!(
        matches!(resp, Response::Committed { .. }),
        "tx A commit failed: {resp:?}"
    );

    // クライアントB: ロールバック（Bの変更は取り消される）
    let resp = send_recv(
        &mut client_b,
        &Request::Rollback {
            tx_id: tx_b,
            session_token: None,
        },
    )
    .await;
    assert!(
        matches!(resp, Response::RolledBack { .. }),
        "tx B rollback failed: {resp:?}"
    );

    // 確認: A のノードのみ残り、B のノードはロールバックされている
    let mut checker = TcpStream::connect(addr).await.unwrap();
    let resp = send_recv(
        &mut checker,
        &Request::Query {
            query: "MATCH (n:TxIsolate) RETURN n.owner".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    match resp {
        Response::Result { rows } => {
            // A がコミット済みなので少なくとも1件、B はロールバック済みなので "B" は含まれない
            assert!(
                rows.iter()
                    .any(|r| r.get("n.owner").and_then(|v| v.as_str()) == Some("A")),
                "A's committed node should exist"
            );
            assert!(
                !rows
                    .iter()
                    .any(|r| r.get("n.owner").and_then(|v| v.as_str()) == Some("B")),
                "B's rolled-back node should not exist"
            );
        }
        other => panic!("expected Result, got {other:?}"),
    }
}

#[tokio::test]
async fn integration_pipeline_multiple_queries_single_connection() {
    // 1接続で複数クエリを順次送信し、サーバーが正しく処理できることを確認する
    let addr = start_test_server().await;
    let mut stream = TcpStream::connect(addr).await.unwrap();

    // 10件の CREATE を同一接続で連続送信
    for i in 0..10 {
        let resp = send_recv(
            &mut stream,
            &Request::Query {
                query: format!("CREATE (n:Pipeline {{idx: {}}}) RETURN n", i),
                params: HashMap::new(),
                tx_id: None,
                session_token: None,
            },
        )
        .await;
        assert!(
            matches!(resp, Response::Result { .. }),
            "CREATE #{} failed: {:?}",
            i,
            resp
        );
    }

    // 同じ接続で MATCH して件数を確認
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "MATCH (n:Pipeline) RETURN n.idx".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    match resp {
        Response::Result { rows } => {
            assert_eq!(rows.len(), 10, "10件のノードが作成されているべき");
        }
        other => panic!("expected Result, got {:?}", other),
    }

    // Ping も挟んで接続が生きていることを確認
    let resp = send_recv(&mut stream, &Request::Ping).await;
    assert!(matches!(resp, Response::Pong));

    // さらに別クエリを続けて送れること
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "MATCH (n:Pipeline {idx: 5}) RETURN n.idx".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    match resp {
        Response::Result { rows } => {
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].get("n.idx").and_then(|v| v.as_i64()), Some(5));
        }
        other => panic!("expected Result, got {:?}", other),
    }
}
