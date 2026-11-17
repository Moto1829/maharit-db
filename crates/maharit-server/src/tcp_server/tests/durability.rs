//! WAL とスナップショットからの復旧、パラメータ、スキーマの保持。

use super::*;

#[tokio::test]
async fn test_durable_wal_survives_crash_across_checkpoint() {
    let dir = std::env::temp_dir().join(format!("maharit_durable_srv_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (addr, server) = start_durable_server(&dir).await;
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let run = |q: &str| Request::Query {
        query: q.to_string(),
        params: HashMap::new(),
        tx_id: None,
        session_token: None,
    };
    for q in [
        "UNWIND range(1, 6) AS i CREATE (:N {i: i})",
        "MATCH (n:N) WHERE n.i IN [2, 3] DETACH DELETE n",
        "CREATE (:D {d: date('2024-02-29'), t: datetime('2026-01-01T00:00:00Z')})",
    ] {
        let resp = send_recv(&mut stream, &run(q)).await;
        assert!(matches!(resp, Response::Result { .. }), "{q}: {resp:?}");
    }
    // Checkpoint mid-run: the live graph has sparse IDs now.
    server.checkpointer().unwrap().checkpoint().await.unwrap();
    for q in [
        "MATCH (a:N {i: 6}), (b:N {i: 1}) CREATE (a)-[:R {w: 1.5}]->(b)",
        "MATCH (n:N {i: 4}) SET n:Extra, n.name = 'four' REMOVE n.i",
        "CREATE (:N {i: 7})",
    ] {
        let resp = send_recv(&mut stream, &run(q)).await;
        assert!(matches!(resp, Response::Result { .. }), "{q}: {resp:?}");
    }
    // Rolled-back changes must not come back either.
    let Response::TransactionBegun { tx_id } = send_recv(
        &mut stream,
        &Request::BeginTransaction {
            read_only: false,
            session_token: None,
        },
    )
    .await
    else {
        panic!("begin failed")
    };
    let mut req = run("MATCH (n:N {i: 1}) SET n.i = 100");
    if let Request::Query { tx_id: t, .. } = &mut req {
        *t = Some(tx_id);
    }
    assert!(matches!(
        send_recv(&mut stream, &req).await,
        Response::Result { .. }
    ));
    assert!(matches!(
        send_recv(
            &mut stream,
            &Request::Rollback {
                tx_id,
                session_token: None
            }
        )
        .await,
        Response::RolledBack { .. }
    ));

    // "Crash": rebuild from disk without a final save and compare.
    let recovered = recover_from_disk(&dir);
    assert_eq!(
        graph_fingerprint(&recovered),
        graph_fingerprint(&server.graph_arc())
    );
    assert_eq!(recovered.edge_count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_request_params_deserialization() {
    let json = r#"{"type": "query", "query": "RETURN $x", "params": {"x": 1, "s": "a\"b", "l": [1.5, null], "m": {"k": true}}}"#;
    let request: Request = serde_json::from_str(json).unwrap();
    let Request::Query { params, .. } = request else {
        panic!("expected Query");
    };
    let params = params_from_json(&params);
    assert_eq!(params["x"], Value::Int(1));
    assert_eq!(params["s"], Value::String("a\"b".to_string()));
    assert_eq!(
        params["l"],
        Value::List(vec![Value::Float(1.5), Value::Null])
    );
    assert_eq!(
        params["m"],
        Value::Map(HashMap::from([("k".to_string(), Value::Bool(true))]))
    );

    // Omitted params default to empty and are not serialized.
    let request: Request =
        serde_json::from_str(r#"{"type": "query", "query": "RETURN 1"}"#).unwrap();
    let out = serde_json::to_string(&request).unwrap();
    assert!(!out.contains("params"), "{out}");
}

#[tokio::test]
async fn test_query_params_over_tcp() {
    let addr = start_test_server().await;
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let q = |query: &str, params: serde_json::Value, tx_id: Option<u64>| Request::Query {
        query: query.to_string(),
        params: serde_json::from_value(params).unwrap(),
        tx_id,
        session_token: None,
    };

    // A value containing quotes is data, not query text.
    let evil = "x\"}) DETACH DELETE n //";
    let resp = send_recv(
        &mut stream,
        &q(
            "CREATE (:P {name: $name, age: $age})",
            serde_json::json!({"name": evil, "age": 30}),
            None,
        ),
    )
    .await;
    assert!(matches!(resp, Response::Result { .. }), "{resp:?}");

    let resp = send_recv(
        &mut stream,
        &q(
            "MATCH (n:P) WHERE n.name = $name AND n.age >= $min RETURN n.name AS name, n.age AS age",
            serde_json::json!({"name": evil, "min": 18}),
            None,
        ),
    )
    .await;
    let Response::Result { rows } = resp else {
        panic!("expected Result, got {resp:?}");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], serde_json::json!(evil));
    assert_eq!(rows[0]["age"], serde_json::json!(30));

    // Inside a transaction.
    let Response::TransactionBegun { tx_id } = send_recv(
        &mut stream,
        &Request::BeginTransaction {
            read_only: false,
            session_token: None,
        },
    )
    .await
    else {
        panic!("begin failed");
    };
    let resp = send_recv(
        &mut stream,
        &q(
            "MATCH (n:P) SET n.age = $age",
            serde_json::json!({"age": 31}),
            Some(tx_id),
        ),
    )
    .await;
    assert!(matches!(resp, Response::Result { .. }), "{resp:?}");

    // A missing parameter is an error, not null.
    let resp = send_recv(
        &mut stream,
        &q("RETURN $missing AS v", serde_json::json!({}), None),
    )
    .await;
    assert!(matches!(resp, Response::Error { .. }), "{resp:?}");
}

#[tokio::test]
async fn test_constraint_persists_across_queries() {
    // 制約が複数クエリをまたいで永続化されることを確認する
    let addr = start_test_server().await;
    let mut stream = TcpStream::connect(addr).await.unwrap();

    // CREATE CONSTRAINT
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "CREATE CONSTRAINT unique_test_id FOR (n:TestItem) REQUIRE n.id IS UNIQUE"
                .to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    assert!(
        matches!(resp, Response::Result { .. }),
        "CREATE CONSTRAINT failed: {:?}",
        resp
    );

    // 最初のノード作成（成功するはず）
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "CREATE (:TestItem {id: 'item-1'})".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    assert!(
        matches!(resp, Response::Result { .. }),
        "First CREATE failed: {:?}",
        resp
    );

    // 重複 id で2回目の作成（制約違反でエラーになるはず）
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "CREATE (:TestItem {id: 'item-1'})".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    assert!(
        matches!(resp, Response::Error { .. }),
        "Expected constraint violation error, got: {:?}",
        resp
    );

    // SHOW CONSTRAINTS で登録済みを確認
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "SHOW CONSTRAINTS".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    match resp {
        Response::Result { rows } => {
            assert!(
                !rows.is_empty(),
                "SHOW CONSTRAINTS should return at least one row"
            );
        }
        other => panic!("SHOW CONSTRAINTS failed: {:?}", other),
    }
}

#[tokio::test]
async fn test_fulltext_index_persists_across_queries() {
    // フルテキストインデックスが複数クエリをまたいで永続化されることを確認する
    let addr = start_test_server().await;
    let mut stream = TcpStream::connect(addr).await.unwrap();

    // CREATE FULLTEXT INDEX
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "CREATE FULLTEXT INDEX ft_test_body FOR (a:Article) ON (a.body)".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    assert!(
        matches!(resp, Response::Result { .. }),
        "CREATE FULLTEXT INDEX failed: {:?}",
        resp
    );

    // DROP FULLTEXT INDEX（永続化されていれば成功するはず）
    let resp = send_recv(
        &mut stream,
        &Request::Query {
            query: "DROP FULLTEXT INDEX ft_test_body".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await;
    assert!(
        matches!(resp, Response::Result { .. }),
        "DROP FULLTEXT INDEX failed (index not persisted): {:?}",
        resp
    );
}

// ── TLS ──────────────────────────────────────────────────────────────────

#[test]
fn loopback_bind_detection() {
    assert!(is_loopback_bind("127.0.0.1:7687"));
    assert!(is_loopback_bind("localhost:7687"));
    assert!(is_loopback_bind("[::1]:7687"));
    assert!(!is_loopback_bind("0.0.0.0:7687"));
    assert!(!is_loopback_bind("[::]:7687"));
    assert!(!is_loopback_bind("192.168.1.10:7687"));
}
