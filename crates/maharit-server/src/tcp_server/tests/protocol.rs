//! 権限判定、リクエスト・レスポンスの直列化、サーバーの設定と WAL の記録（TCP 接続なし）。

use super::*;

#[test]
fn test_authorize_query_readonly_allows_read() {
    // 読み取りクエリは ReadOnly ロールで許可される。
    assert!(authorize_query(Role::ReadOnly, "MATCH (n) RETURN n").is_none());
}

#[test]
fn test_authorize_query_readonly_denies_write() {
    // 書き込みクエリは ReadOnly ロールでは拒否される。
    let resp = authorize_query(Role::ReadOnly, "CREATE (n:Person {name: 'x'})");
    assert!(matches!(resp, Some(Response::AuthError { .. })));
}

#[test]
fn test_authorize_query_readwrite_allows_write() {
    assert!(authorize_query(Role::ReadWrite, "CREATE (n:Person {name: 'x'})").is_none());
}

#[test]
fn test_authorize_query_admin_allows_write() {
    assert!(authorize_query(Role::Admin, "CREATE (n:Person {name: 'x'})").is_none());
}

#[test]
fn test_authorize_query_unparseable_skips_check() {
    // パース不能なクエリは権限判定をスキップ（実行側でパースエラーを返す）。
    assert!(authorize_query(Role::ReadOnly, "NOT A VALID QUERY @#$").is_none());
}

#[test]
fn test_check_session_disabled_returns_admin() {
    let auth = Arc::new(Mutex::new(crate::auth::AuthManager::new()));
    // require_auth=false のときは常に Admin 相当で通す。
    assert!(matches!(
        check_session(false, &auth, &None),
        Ok(Role::Admin)
    ));
}

#[test]
fn test_check_session_enabled_missing_token() {
    let auth = Arc::new(Mutex::new(crate::auth::AuthManager::new()));
    let result = check_session(true, &auth, &None);
    assert!(matches!(result, Err(Response::AuthError { .. })));
}

#[test]
fn test_request_parsing() {
    let json = r#"{"type": "query", "query": "MATCH (n) RETURN n"}"#;
    let request: Request = serde_json::from_str(json).unwrap();
    match request {
        Request::Query { query, tx_id, .. } => {
            assert_eq!(query, "MATCH (n) RETURN n");
            assert!(tx_id.is_none());
        }
        _ => panic!("Expected Query request"),
    }
}

#[test]
fn test_request_parsing_with_tx_id() {
    let json = r#"{"type": "query", "query": "MATCH (n) RETURN n", "txId": 42}"#;
    let request: Request = serde_json::from_str(json).unwrap();
    match request {
        Request::Query { query, tx_id, .. } => {
            assert_eq!(query, "MATCH (n) RETURN n");
            assert_eq!(tx_id, Some(42));
        }
        _ => panic!("Expected Query request"),
    }
}

#[test]
fn test_begin_transaction_request() {
    let json = r#"{"type": "begin"}"#;
    let request: Request = serde_json::from_str(json).unwrap();
    match request {
        Request::BeginTransaction { read_only, .. } => {
            assert!(!read_only);
        }
        _ => panic!("Expected BeginTransaction request"),
    }
}

#[test]
fn test_begin_read_only_transaction_request() {
    let json = r#"{"type": "begin", "readOnly": true}"#;
    let request: Request = serde_json::from_str(json).unwrap();
    match request {
        Request::BeginTransaction { read_only, .. } => {
            assert!(read_only);
        }
        _ => panic!("Expected BeginTransaction request"),
    }
}

#[test]
fn test_commit_request() {
    let json = r#"{"type": "commit", "txId": 123}"#;
    let request: Request = serde_json::from_str(json).unwrap();
    match request {
        Request::Commit { tx_id, .. } => {
            assert_eq!(tx_id, 123);
        }
        _ => panic!("Expected Commit request"),
    }
}

#[test]
fn test_rollback_request() {
    let json = r#"{"type": "rollback", "txId": 456}"#;
    let request: Request = serde_json::from_str(json).unwrap();
    match request {
        Request::Rollback { tx_id, .. } => {
            assert_eq!(tx_id, 456);
        }
        _ => panic!("Expected Rollback request"),
    }
}

#[test]
fn test_login_request_parsing() {
    let json = r#"{"type": "login", "username": "admin", "password": "admin"}"#;
    let request: Request = serde_json::from_str(json).unwrap();
    match request {
        Request::Login { username, password } => {
            assert_eq!(username, "admin");
            assert_eq!(password, "admin");
        }
        _ => panic!("Expected Login request"),
    }
}

#[test]
fn test_query_with_session_token_parsing() {
    let json = r#"{"type": "query", "query": "MATCH (n) RETURN n", "sessionToken": "abc-123"}"#;
    let request: Request = serde_json::from_str(json).unwrap();
    match request {
        Request::Query {
            query,
            tx_id,
            session_token,
            ..
        } => {
            assert_eq!(query, "MATCH (n) RETURN n");
            assert!(tx_id.is_none());
            assert_eq!(session_token.as_deref(), Some("abc-123"));
        }
        _ => panic!("Expected Query request"),
    }
}

#[test]
fn test_check_session_when_auth_disabled() {
    let auth = Arc::new(Mutex::new(crate::auth::AuthManager::new()));
    // require_auth=false ならトークン無しでも Ok(Admin) を返す
    assert!(check_session(false, &auth, &None).is_ok());
    assert!(check_session(false, &auth, &Some("garbage".to_string())).is_ok());
}

#[test]
fn test_check_session_when_auth_enabled() {
    let auth = Arc::new(Mutex::new(crate::auth::AuthManager::new()));

    // require_auth=true でトークン無し → AuthError
    let resp = check_session(true, &auth, &None);
    assert!(matches!(resp, Err(Response::AuthError { .. })));

    // require_auth=true で無効なトークン → AuthError
    let resp = check_session(true, &auth, &Some("garbage".to_string()));
    assert!(matches!(resp, Err(Response::AuthError { .. })));

    // 正規ログイン後のトークンなら通り、Admin ロールが返る
    let token = {
        let mut mgr = auth.lock().unwrap();
        mgr.authenticate("admin", "admin").unwrap()
    };
    assert!(matches!(
        check_session(true, &auth, &Some(token)),
        Ok(Role::Admin)
    ));
}

#[test]
fn test_auth_error_response_serialization() {
    let resp = Response::AuthError {
        message: "missing sessionToken".to_string(),
    };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("\"type\":\"authError\""));
    assert!(json.contains("missing sessionToken"));
}

#[test]
fn test_logged_in_response_serialization() {
    let resp = Response::LoggedIn {
        session_token: "tok-1".to_string(),
        role: "admin".to_string(),
        expires_at: 1_700_000_000,
    };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("\"type\":\"loggedIn\""));
    assert!(json.contains("\"sessionToken\":\"tok-1\""));
    assert!(json.contains("\"role\":\"admin\""));
    assert!(json.contains("\"expiresAt\":1700000000"));
}

#[test]
fn test_transaction_begun_response() {
    let response = Response::TransactionBegun { tx_id: 42 };
    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("\"type\":\"transactionBegun\""));
    assert!(json.contains("\"txId\":42"));
}

#[test]
fn test_committed_response() {
    let response = Response::Committed { tx_id: 42 };
    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("\"type\":\"committed\""));
    assert!(json.contains("\"txId\":42"));
}

#[test]
fn test_rolled_back_response() {
    let response = Response::RolledBack { tx_id: 42 };
    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("\"type\":\"rolledBack\""));
    assert!(json.contains("\"txId\":42"));
}

#[test]
fn test_ping_request() {
    let json = r#"{"type": "ping"}"#;
    let request: Request = serde_json::from_str(json).unwrap();
    assert!(matches!(request, Request::Ping));
}

#[test]
fn test_response_serialization() {
    let response = Response::Pong;
    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("\"type\":\"pong\""));
}

#[test]
fn test_result_response() {
    let mut row = HashMap::new();
    row.insert("name".to_string(), serde_json::json!("Alice"));
    let response = Response::Result { rows: vec![row] };
    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("\"type\":\"result\""));
    assert!(json.contains("Alice"));
}

#[test]
fn test_error_response() {
    let response = Response::Error {
        message: "Something went wrong".to_string(),
    };
    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("\"type\":\"error\""));
    assert!(json.contains("Something went wrong"));
}

#[test]
fn test_default_config() {
    let config = ServerConfig::default();
    assert_eq!(config.bind_address, "127.0.0.1:7687");
    assert_eq!(config.max_connections, 100);
}

#[tokio::test]
async fn test_server_creation() {
    let config = ServerConfig::default();
    let server = TcpServer::new(config);
    assert_eq!(server.stats.current_connections.load(Ordering::SeqCst), 0);
}

#[test]
fn test_stream_query_request() {
    let json = r#"{"type": "streamQuery", "query": "MATCH (n) RETURN n", "chunkSize": 50}"#;
    let request: Request = serde_json::from_str(json).unwrap();
    match request {
        Request::StreamQuery {
            query,
            tx_id,
            chunk_size,
            session_token: None,
            ..
        } => {
            assert_eq!(query, "MATCH (n) RETURN n");
            assert!(tx_id.is_none());
            assert_eq!(chunk_size, 50);
        }
        _ => panic!("Expected StreamQuery request"),
    }
}

#[test]
fn test_stream_query_request_default_chunk_size() {
    let json = r#"{"type": "streamQuery", "query": "MATCH (n) RETURN n"}"#;
    let request: Request = serde_json::from_str(json).unwrap();
    match request {
        Request::StreamQuery { chunk_size, .. } => {
            assert_eq!(chunk_size, DEFAULT_CHUNK_SIZE);
        }
        _ => panic!("Expected StreamQuery request"),
    }
}

#[test]
fn test_stream_start_response() {
    let response = Response::StreamStart {
        stream_id: 1,
        total_rows: Some(100),
    };
    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("\"type\":\"streamStart\""));
    assert!(json.contains("\"streamId\":1"));
    assert!(json.contains("\"totalRows\":100"));
}

#[test]
fn test_stream_chunk_response() {
    let mut row = HashMap::new();
    row.insert("name".to_string(), serde_json::json!("Alice"));
    let response = Response::StreamChunk {
        stream_id: 1,
        chunk_index: 0,
        rows: vec![row],
    };
    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("\"type\":\"streamChunk\""));
    assert!(json.contains("\"streamId\":1"));
    assert!(json.contains("\"chunkIndex\":0"));
    assert!(json.contains("Alice"));
}

#[test]
fn test_stream_end_response() {
    let response = Response::StreamEnd {
        stream_id: 1,
        total_rows: 100,
    };
    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("\"type\":\"streamEnd\""));
    assert!(json.contains("\"streamId\":1"));
    assert!(json.contains("\"totalRows\":100"));
}

#[test]
fn test_next_stream_id() {
    let stats = ServerStats::default();
    assert_eq!(stats.next_stream_id(), 0);
    assert_eq!(stats.next_stream_id(), 1);
    assert_eq!(stats.next_stream_id(), 2);
}

// ── Replication integration ──────────────────────────────────────────────

#[test]
fn test_tcp_server_with_replication_builder() {
    use crate::replication::{LeaderReplicationManager, ReplicationConfig};

    let config = ReplicationConfig::default();
    let repl = Arc::new(LeaderReplicationManager::new(config));
    let server = TcpServer::new(ServerConfig::default()).with_replication(Arc::clone(&repl));
    assert!(server.replication.read().unwrap().is_some());
}

#[tokio::test]
async fn test_emit_wal_entries_appends_in_order() {
    use crate::replication::{LeaderReplicationManager, ReplicationConfig};

    let repl = Arc::new(LeaderReplicationManager::new(ReplicationConfig::default()));
    emit_wal_entries(
        vec![
            WalEntryData::CreateNode {
                node_id: 0,
                labels: vec!["Person".to_string()],
            },
            WalEntryData::DeleteNode { node_id: 0 },
        ],
        &repl,
    )
    .await;

    assert_eq!(repl.get_stats().current_lsn, 2);
}

/// SET / REMOVE / label changes on existing nodes must be captured for
/// replication (the old before/after ID-set diff missed them entirely).
#[test]
fn write_execution_records_updates_to_existing_elements() {
    let graph = ConcurrentGraph::new();
    let managers = SharedManagers::new();
    let parse = |q: &str| Parser::new(q).unwrap().parse().unwrap();

    let (r, _) = managers.execute(&graph, parse("CREATE (:P {name: 'a', age: 1})"), true, true);
    r.unwrap();

    let (r, wal) = managers.execute(&graph, parse("MATCH (n:P) SET n.age = 2, n:Q"), true, true);
    r.unwrap();
    assert!(wal.iter().any(|e| matches!(
        e,
        WalEntryData::SetProperty { key, value, .. } if key == "age" && value == "2"
    )));
    assert!(wal.iter().any(|e| matches!(
        e,
        WalEntryData::AddLabel { label, .. } if label == "Q"
    )));

    let (r, wal) = managers.execute(&graph, parse("MATCH (n:P) REMOVE n.name, n:P"), true, true);
    r.unwrap();
    assert!(wal.iter().any(|e| matches!(
        e,
        WalEntryData::RemoveProperty { key, .. } if key == "name"
    )));
    assert!(wal.iter().any(|e| matches!(
        e,
        WalEntryData::RemoveLabel { label, .. } if label == "P"
    )));

    // Without replication nothing is recorded.
    let (r, wal) = managers.execute(&graph, parse("MATCH (n:P) SET n.age = 3"), true, false);
    r.unwrap();
    assert!(wal.is_empty());
}

// ── Integration tests (actual TCP socket communication) ──────────────────

/// task193: 読み取りは使いうる管理対象のロックだけを取る。書き込みが索引を握っていても、
/// 索引の張られていないプロパティしか読まない文は待たない。
#[test]
fn reads_skip_locks_they_cannot_use() {
    let graph = ConcurrentGraph::new();
    let managers = SharedManagers::new();
    let parse = |q: &str| Parser::new(q).unwrap().parse().unwrap();
    let (r, _) = managers.execute(&graph, parse("CREATE INDEX ON :P(id)"), true, false);
    r.unwrap();
    let (r, _) = managers.execute(&graph, parse("CREATE (:P {id: 1, name: 'a'})"), true, false);
    r.unwrap();

    let needs = managers.read_needs("MATCH (n:P) WHERE n.name = 'a' RETURN n.name");
    assert_eq!(
        (needs.constraints, needs.fulltext, needs.property_index),
        (false, false, false)
    );
    let uses_index = managers.read_needs("MATCH (n:P {id: 1}) RETURN n");
    assert!(uses_index.property_index);
    assert!(managers.read_needs("SHOW INDEXES").property_index);
    assert!(managers.read_needs("SHOW CONSTRAINTS").constraints);
    assert!(
        managers
            .read_needs("CALL db.index.fulltext.search('ft', 'x') YIELD node RETURN node")
            .fulltext
    );

    // 書き込みが 3 つのロックを全て握っている間でも、読み取りは終わる
    let _cm = managers.constraints.write().unwrap();
    let _fm = managers.fulltext.write().unwrap();
    let _pi = managers.property_index.write().unwrap();
    let q = "MATCH (n:P) WHERE n.name = 'a' RETURN n.name";
    let needs = managers.read_needs(q);
    let (r, _) = managers.execute_with_params(
        &graph,
        parse(q),
        HashMap::new(),
        false,
        false,
        None,
        Some(needs),
    );
    assert_eq!(r.unwrap().rows.len(), 1);
}
