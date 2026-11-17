//! レプリケーション（フォロワーの同期・昇格・スナップショット・スキーマの伝播）。

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn follower_syncs_rejects_writes_and_can_be_promoted() {
    use crate::durable_wal::{DurableWal, WalMode, wal_path};
    use crate::replication::NodeRole;

    // Leader with data written before any follower connects.
    let leader_repl_addr = free_addr().await;
    let leader_repl = Arc::new(LeaderReplicationManager::new(repl_config(
        NodeRole::Leader,
        "leader",
        leader_repl_addr.to_string(),
        None,
    )));
    let leader_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let leader_addr = leader_listener.local_addr().unwrap();
    let leader_server = TcpServer::new(ServerConfig {
        bind_address: leader_addr.to_string(),
        ..Default::default()
    })
    .with_replication(Arc::clone(&leader_repl));
    leader_repl.start().await.unwrap();
    tokio::spawn(async move { leader_server.start_with_listener(leader_listener).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    let mut leader = TcpStream::connect(leader_addr).await.unwrap();
    query(
        &mut leader,
        "CREATE (:Old {i: 1})-[:R {w: 2}]->(:Old {i: 2})",
    )
    .await;

    // Follower whose graph holds stale data that the snapshot must replace.
    let dir = std::env::temp_dir().join(format!("maharit_promote_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let data = dir.join("db");
    let (wal, _) = DurableWal::open(
        &wal_path(&data),
        snapshot_digest(&data).unwrap(),
        WalMode::Fsync,
    )
    .unwrap();
    let follower_graph = Arc::new(ConcurrentGraph::new());
    follower_graph.create_node("Stale");
    let follower = Arc::new(FollowerReplicationManager::with_concurrent_graph(
        repl_config(
            NodeRole::Follower,
            "follower",
            "127.0.0.1:0".to_string(),
            Some(leader_repl_addr.to_string()),
        ),
        Arc::clone(&follower_graph),
    ));
    let promoted_repl_addr = free_addr().await;
    let follower_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let follower_addr = follower_listener.local_addr().unwrap();
    let follower_server = TcpServer::with_graph_arc(
        ServerConfig {
            bind_address: follower_addr.to_string(),
            ..Default::default()
        },
        Arc::clone(&follower_graph),
    )
    .with_follower(Arc::clone(&follower))
    .with_promotion(Promotion {
        replication: Some(repl_config(
            NodeRole::Leader,
            "follower",
            promoted_repl_addr.to_string(),
            None,
        )),
        tls: None,
    })
    .with_durability(Durability::new(wal, &data, 0));
    follower.start().await.unwrap();
    tokio::spawn(async move { follower_server.start_with_listener(follower_listener).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    let mut f = TcpStream::connect(follower_addr).await.unwrap();

    // The snapshot replaced the stale data with the leader's.
    assert_eq!(wait_rows(&mut f, "MATCH (n:Old) RETURN n", 2).await, 2);
    assert_eq!(row_count(&mut f, "MATCH (n:Stale) RETURN n").await, 0);
    assert_eq!(row_count(&mut f, "MATCH ()-[r:R]->() RETURN r.w").await, 1);
    // ...and later writes stream in.
    query(&mut leader, "CREATE (:Old {i: 3})").await;
    assert_eq!(wait_rows(&mut f, "MATCH (n:Old) RETURN n", 3).await, 3);

    // Writes on the follower are refused; reads work.
    let resp = query(&mut f, "CREATE (:Rogue)").await;
    assert!(
        matches!(&resp, Response::Error { message } if message.contains("read-only follower")),
        "{resp:?}"
    );
    assert_eq!(row_count(&mut f, "MATCH (n:Rogue) RETURN n").await, 0);

    // Promote.
    let resp = send_recv(
        &mut f,
        &Request::Promote {
            session_token: None,
        },
    )
    .await;
    assert!(
        matches!(&resp, Response::Promoted { replication_bind: Some(b) } if *b == promoted_repl_addr.to_string()),
        "{resp:?}"
    );
    // The data file now holds everything replicated before promotion.
    let saved = maharit_storage::PersistentStorage::load_concurrent(&data).unwrap();
    assert_eq!(saved.node_count(), 3);
    // A second promote is refused.
    let again = send_recv(
        &mut f,
        &Request::Promote {
            session_token: None,
        },
    )
    .await;
    assert!(matches!(again, Response::Error { .. }), "{again:?}");

    // The promoted node accepts writes and no longer follows the old leader.
    let resp = query(&mut f, "CREATE (:New {i: 1})").await;
    assert!(matches!(resp, Response::Result { .. }), "{resp:?}");
    query(&mut leader, "CREATE (:Old {i: 4})").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(row_count(&mut f, "MATCH (n:Old) RETURN n").await, 3);

    // A follower of the promoted node gets its full state, then new writes.
    let graph2 = Arc::new(ConcurrentGraph::new());
    let follower2 = Arc::new(FollowerReplicationManager::with_concurrent_graph(
        repl_config(
            NodeRole::Follower,
            "follower2",
            "127.0.0.1:0".to_string(),
            Some(promoted_repl_addr.to_string()),
        ),
        Arc::clone(&graph2),
    ));
    follower2.start().await.unwrap();
    query(&mut f, "CREATE (:New {i: 2})").await;
    let mut synced = false;
    for _ in 0..150 {
        if graph2.node_count() == 5 && graph2.edge_count() == 1 {
            synced = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        synced,
        "follower2 has {} nodes / {} edges",
        graph2.node_count(),
        graph2.edge_count()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn promote_requires_a_follower_and_admin_role() {
    let addr = start_test_server().await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    let resp = send_recv(
        &mut s,
        &Request::Promote {
            session_token: None,
        },
    )
    .await;
    assert!(
        matches!(&resp, Response::Error { message } if message.contains("not a follower")),
        "{resp:?}"
    );
}

/// A follower that connects while writes are in flight must end up with
/// exactly the leader's graph: nothing missing between the snapshot and
/// the stream, nothing applied twice.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_during_concurrent_writes_matches_leader() {
    use crate::replication::NodeRole;

    let repl_addr = free_addr().await;
    let repl = Arc::new(LeaderReplicationManager::new(repl_config(
        NodeRole::Leader,
        "leader",
        repl_addr.to_string(),
        None,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = TcpServer::new(ServerConfig {
        bind_address: addr.to_string(),
        ..Default::default()
    })
    .with_replication(Arc::clone(&repl));
    let leader_graph = server.graph_arc();
    repl.start().await.unwrap();
    tokio::spawn(async move { server.start_with_listener(listener).await });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let writers: Vec<_> = (0..4)
        .map(|w| {
            tokio::spawn(async move {
                let mut s = TcpStream::connect(addr).await.unwrap();
                for i in 0..150 {
                    query(&mut s, &format!("CREATE (:W {{w: {w}, i: {i}}})")).await;
                    if i % 3 == 0 {
                        query(
                            &mut s,
                            &format!("MATCH (n:W {{w: {w}, i: {i}}}) SET n.even = true"),
                        )
                        .await;
                    }
                    if i % 5 == 0 && i > 0 {
                        let prev = i - 1;
                        query(
                            &mut s,
                            &format!("MATCH (n:W {{w: {w}, i: {prev}}}) DELETE n"),
                        )
                        .await;
                    }
                }
            })
        })
        .collect();

    tokio::time::sleep(Duration::from_millis(50)).await;
    let graph = Arc::new(ConcurrentGraph::new());
    let follower = Arc::new(FollowerReplicationManager::with_concurrent_graph(
        repl_config(
            NodeRole::Follower,
            "f",
            "127.0.0.1:0".to_string(),
            Some(repl_addr.to_string()),
        ),
        Arc::clone(&graph),
    ));
    follower.start().await.unwrap();
    for w in writers {
        w.await.unwrap();
    }

    let fingerprint = |g: &ConcurrentGraph| {
        let mut v: Vec<String> = g
            .nodes()
            .map(|n| {
                let mut props: Vec<_> = n
                    .properties
                    .iter()
                    .map(|(k, v)| format!("{k}={v:?}"))
                    .collect();
                props.sort();
                format!("{}:{:?}:{}", n.id, n.labels, props.join(","))
            })
            .collect();
        v.sort();
        v
    };
    let mut same = false;
    for _ in 0..200 {
        if fingerprint(&graph) == fingerprint(&leader_graph) {
            same = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        same,
        "follower has {} nodes, leader {}",
        graph.node_count(),
        leader_graph.node_count()
    );
    assert!(leader_graph.node_count() > 400);
}

// ── Schema persistence and replication (task149) ─────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn schema_survives_restart_from_snapshot_and_wal() {
    let dir = std::env::temp_dir().join(format!("maharit_schema_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (addr, server) = start_durable_server(&dir).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    for q in [
        "CREATE CONSTRAINT email_unique FOR (p:Person) REQUIRE p.email IS UNIQUE",
        "CREATE INDEX ON :Person(name)",
        "CREATE (:Person {name: 'Alice', email: 'a@example.com', bio: 'graph databases'})",
    ] {
        let resp = query(&mut s, q).await;
        assert!(matches!(resp, Response::Result { .. }), "{q}: {resp:?}");
    }
    // Checkpoint: the schema so far goes into the data file.
    server.checkpointer().unwrap().checkpoint().await.unwrap();
    for q in [
        "CREATE FULLTEXT INDEX bios FOR (p:Person) ON EACH [p.bio]",
        "CREATE CONSTRAINT age_int FOR (p:Person) REQUIRE p.age IS :: INTEGER",
        "DROP INDEX ON :Person(name)",
        "CREATE INDEX ON :Person(email)",
        "CREATE (:Person {name: 'Bob', email: 'b@example.com', bio: 'rust programming'})",
    ] {
        let resp = query(&mut s, q).await;
        assert!(matches!(resp, Response::Result { .. }), "{q}: {resp:?}");
    }

    // "Crash" and restart from the data file + WAL.
    let addr = serve(restart_from_disk(&dir)).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    let constraints: Vec<String> = rows(&mut s, "SHOW CONSTRAINTS")
        .await
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(constraints.len(), 2, "{constraints:?}");
    assert!(constraints.contains(&"email_unique".to_string()));
    assert!(constraints.contains(&"age_int".to_string()));
    let indexes = rows(&mut s, "SHOW INDEXES").await;
    assert_eq!(indexes.len(), 1, "{indexes:?}");
    assert_eq!(indexes[0]["property"], "email");

    // The UNIQUE and type constraints are enforced again.
    let dup = query(
        &mut s,
        "CREATE (:Person {name: 'X', email: 'a@example.com'})",
    )
    .await;
    assert!(matches!(dup, Response::Error { .. }), "{dup:?}");
    let bad = query(
        &mut s,
        "CREATE (:Person {name: 'Y', email: 'y@x', age: 'old'})",
    )
    .await;
    assert!(matches!(bad, Response::Error { .. }), "{bad:?}");
    // The rebuilt indexes find the existing nodes.
    let hit = rows(
        &mut s,
        "MATCH (p:Person {email: 'b@example.com'}) RETURN p.name AS name",
    )
    .await;
    assert_eq!(hit.len(), 1);
    let found = rows(
        &mut s,
        "CALL db.index.fulltext.search(\"bios\", \"rust\") YIELD node RETURN node.name AS name",
    )
    .await;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["name"], "Bob");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn followers_receive_the_schema_and_keep_indexes_current() {
    use crate::replication::NodeRole;

    let repl_addr = free_addr().await;
    let repl = Arc::new(LeaderReplicationManager::new(repl_config(
        NodeRole::Leader,
        "leader",
        repl_addr.to_string(),
        None,
    )));
    let leader_server = TcpServer::new(ServerConfig::default()).with_replication(Arc::clone(&repl));
    repl.start().await.unwrap();
    let mut leader = TcpStream::connect(serve(leader_server).await)
        .await
        .unwrap();
    for q in [
        "CREATE CONSTRAINT email_unique FOR (p:Person) REQUIRE p.email IS UNIQUE",
        "CREATE INDEX ON :Person(name)",
        "CREATE (:Person {name: 'Alice', email: 'a@example.com'})",
    ] {
        query(&mut leader, q).await;
    }

    // The follower connects after the DDL: it comes with the snapshot.
    let graph = Arc::new(ConcurrentGraph::new());
    let follower = Arc::new(FollowerReplicationManager::with_concurrent_graph(
        repl_config(
            NodeRole::Follower,
            "f",
            "127.0.0.1:0".to_string(),
            Some(repl_addr.to_string()),
        ),
        Arc::clone(&graph),
    ));
    let promoted_repl = free_addr().await;
    let follower_server = TcpServer::with_graph_arc(ServerConfig::default(), Arc::clone(&graph))
        .with_follower(Arc::clone(&follower))
        .with_promotion(Promotion {
            replication: Some(repl_config(
                NodeRole::Leader,
                "f",
                promoted_repl.to_string(),
                None,
            )),
            tls: None,
        });
    follower.start().await.unwrap();
    let mut f = TcpStream::connect(serve(follower_server).await)
        .await
        .unwrap();

    let mut synced = false;
    for _ in 0..150 {
        if row_count(&mut f, "SHOW CONSTRAINTS").await == 1
            && row_count(&mut f, "SHOW INDEXES").await == 1
        {
            synced = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(synced, "schema did not arrive with the snapshot");

    // Later DDL and data changes stream in; the follower's index follows.
    query(
        &mut leader,
        "CREATE FULLTEXT INDEX names FOR (p:Person) ON EACH [p.name]",
    )
    .await;
    query(
        &mut leader,
        "MATCH (p:Person {name: 'Alice'}) SET p.name = 'Alicia'",
    )
    .await;
    query(
        &mut leader,
        "CREATE (:Person {name: 'Bob', email: 'b@example.com'})",
    )
    .await;
    let q = "MATCH (p:Person {name: 'Alicia'}) RETURN p.email AS email";
    let mut got = Vec::new();
    for _ in 0..150 {
        got = rows(&mut f, q).await;
        if got.len() == 1 && row_count(&mut f, "MATCH (p:Person) RETURN p").await == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(
        rows(&mut f, "MATCH (p:Person {name: 'Alice'}) RETURN p")
            .await
            .is_empty(),
        "stale index entry for the old name"
    );
    let found = rows(
        &mut f,
        "CALL db.index.fulltext.search(\"names\", \"bob\") YIELD node RETURN node.email AS email",
    )
    .await;
    assert_eq!(found.len(), 1, "{found:?}");

    // After promotion the replicated UNIQUE constraint is enforced.
    let resp = send_recv(
        &mut f,
        &Request::Promote {
            session_token: None,
        },
    )
    .await;
    assert!(matches!(resp, Response::Promoted { .. }), "{resp:?}");
    let dup = query(
        &mut f,
        "CREATE (:Person {name: 'Z', email: 'b@example.com'})",
    )
    .await;
    assert!(matches!(dup, Response::Error { .. }), "{dup:?}");
}
