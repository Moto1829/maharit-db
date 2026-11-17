//! tcp_server.rs のテスト。分野ごとのファイルに分け、共通のヘルパーはここに置く。

use super::*;
use crate::auth::Role;
use tokio::net::TcpStream;

mod durability;
mod integration;
mod protocol;
mod replication;
mod tls;

/// Send a request and receive a response over a raw TCP stream.
async fn send_recv<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S, req: &Request) -> Response {
    let json = serde_json::to_vec(req).unwrap();
    let len = json.len() as u32;
    stream.write_all(&len.to_be_bytes()).await.unwrap();
    stream.write_all(&json).await.unwrap();
    stream.flush().await.unwrap();

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await.unwrap();
    let body_len = u32::from_be_bytes(len_buf) as usize;
    let mut body = vec![0u8; body_len];
    stream.read_exact(&mut body).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// Bind to port 0, spawn the server, return the bound address.
async fn start_test_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = TcpServer::new(ServerConfig {
        bind_address: addr.to_string(),
        read_timeout: Duration::from_secs(5),
        write_timeout: Duration::from_secs(5),
        ..Default::default()
    });

    tokio::spawn(async move {
        server.start_with_listener(listener).await.unwrap();
    });

    // Give the server task a moment to enter the accept loop.
    tokio::time::sleep(Duration::from_millis(10)).await;
    addr
}

async fn query(stream: &mut TcpStream, q: &str) -> Response {
    send_recv(
        stream,
        &Request::Query {
            query: q.to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        },
    )
    .await
}

async fn tx_query(stream: &mut TcpStream, tx_id: u64, q: &str) -> Response {
    send_recv(
        stream,
        &Request::Query {
            query: q.to_string(),
            params: HashMap::new(),
            tx_id: Some(tx_id),
            session_token: None,
        },
    )
    .await
}

async fn begin(stream: &mut TcpStream) -> u64 {
    match send_recv(
        stream,
        &Request::BeginTransaction {
            read_only: false,
            session_token: None,
        },
    )
    .await
    {
        Response::TransactionBegun { tx_id } => tx_id,
        other => panic!("begin failed: {other:?}"),
    }
}

async fn rollback(stream: &mut TcpStream, tx_id: u64) {
    let resp = send_recv(
        stream,
        &Request::Rollback {
            tx_id,
            session_token: None,
        },
    )
    .await;
    assert!(
        matches!(resp, Response::RolledBack { .. }),
        "rollback failed: {resp:?}"
    );
}

async fn row_count(stream: &mut TcpStream, q: &str) -> usize {
    match query(stream, q).await {
        Response::Result { rows } => rows.len(),
        other => panic!("query failed: {q}: {other:?}"),
    }
}

/// max_connections を指定してテストサーバーを起動する。
async fn start_test_server_with_config(max_connections: usize) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = TcpServer::new(ServerConfig {
        bind_address: addr.to_string(),
        max_connections,
        read_timeout: Duration::from_secs(5),
        write_timeout: Duration::from_secs(5),
        require_auth: false,
    });

    tokio::spawn(async move {
        server.start_with_listener(listener).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(10)).await;
    addr
}

/// Start a server with an on-disk WAL in `dir`; returns its address and
/// the server handle (for checkpoints and the live graph).
async fn start_durable_server(dir: &std::path::Path) -> (std::net::SocketAddr, Arc<TcpServer>) {
    use crate::durable_wal::{DurableWal, WalMode, wal_path};
    let data = dir.join("db");
    let (wal, recovery) = DurableWal::open(
        &wal_path(&data),
        snapshot_digest(&data).unwrap(),
        WalMode::Fsync,
    )
    .unwrap();
    assert!(recovery.batches.is_empty());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = Arc::new(
        TcpServer::new(ServerConfig {
            bind_address: addr.to_string(),
            ..Default::default()
        })
        .with_durability(Durability::new(wal, &data, 0)),
    );
    let s = Arc::clone(&server);
    tokio::spawn(async move { s.start_with_listener(listener).await.unwrap() });
    tokio::time::sleep(Duration::from_millis(10)).await;
    (addr, server)
}

/// What a restart would see: the snapshot plus the replayed WAL.
fn recover_from_disk(dir: &std::path::Path) -> ConcurrentGraph {
    use crate::durable_wal::{DurableWal, WalMode, wal_path};
    let data = dir.join("db");
    let graph = if data.exists() {
        maharit_storage::PersistentStorage::load_concurrent(&data).unwrap()
    } else {
        ConcurrentGraph::new()
    };
    let (_, recovery) = DurableWal::open(
        &wal_path(&data),
        snapshot_digest(&data).unwrap(),
        WalMode::Os,
    )
    .unwrap();
    for entry in recovery.batches.iter().flatten() {
        crate::replication::apply_wal_entry(&graph, entry);
    }
    graph
}

/// Nodes and edges by ID, for comparing two graphs.
fn graph_fingerprint(g: &ConcurrentGraph) -> (Vec<String>, Vec<String>) {
    let mut nodes: Vec<String> = g
        .nodes()
        .map(|n| {
            let n = n.value();
            let mut props: Vec<_> = n
                .properties
                .iter()
                .map(|(k, v)| format!("{k}={v:?}"))
                .collect();
            props.sort();
            let mut labels = n.labels.clone();
            labels.sort();
            format!("{} {:?} {:?}", n.id, labels, props)
        })
        .collect();
    nodes.sort();
    let mut edges: Vec<String> = g
        .edges()
        .map(|e| {
            let e = e.value();
            let mut props: Vec<_> = e
                .properties
                .iter()
                .map(|(k, v)| format!("{k}={v:?}"))
                .collect();
            props.sort();
            format!("{} {}->{} {} {:?}", e.id, e.from, e.to, e.label, props)
        })
        .collect();
    edges.sort();
    (nodes, edges)
}

/// Start a TLS server whose certificate and key are read from the given
/// files.
async fn start_tls_server(cert: &str, key: &str) -> (std::net::SocketAddr, ReloadableTlsAcceptor) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let tls = ReloadableTlsAcceptor::new(crate::tls::TlsConfig::new(cert, key)).unwrap();
    let server = TcpServer::new(ServerConfig {
        bind_address: addr.to_string(),
        read_timeout: Duration::from_secs(5),
        write_timeout: Duration::from_secs(5),
        ..Default::default()
    })
    .with_tls(tls.clone());
    tokio::spawn(async move {
        server.start_with_listener(listener).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    (addr, tls)
}

/// Open a TLS connection that trusts only the CA certificate in `ca`.
async fn tls_connect(
    addr: std::net::SocketAddr,
    ca: &str,
) -> std::io::Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let config = crate::tls::build_client_config(Some(ca), false).unwrap();
    let connector = tokio_rustls::TlsConnector::from(config);
    let tcp = TcpStream::connect(addr).await?;
    let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
    connector.connect(name, tcp).await
}

/// A free localhost port (bound and released).
async fn free_addr() -> std::net::SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap()
}

fn repl_config(
    role: crate::replication::NodeRole,
    id: &str,
    bind: String,
    leader: Option<String>,
) -> ReplicationConfig {
    ReplicationConfig {
        role,
        node_id: id.to_string(),
        replication_bind_address: bind,
        leader_address: leader,
        heartbeat_interval_secs: 1,
        heartbeat_timeout_secs: 5,
        shared_secret: None,
    }
}

async fn wait_rows(stream: &mut TcpStream, q: &str, n: usize) -> usize {
    let mut got = 0;
    for _ in 0..150 {
        got = row_count(stream, q).await;
        if got == n {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    got
}

/// Load the data file and replay the WAL like `maharit server` does,
/// returning a server (not started) with the recovered schema installed.
fn restart_from_disk(dir: &std::path::Path) -> TcpServer {
    use crate::durable_wal::{DurableWal, WalMode, wal_path};
    let data = dir.join("db");
    let (graph, mut schema) = if data.exists() {
        let (g, json) =
            maharit_storage::PersistentStorage::load_concurrent_with_schema(&data).unwrap();
        let schema = json
            .map(|j| serde_json::from_str(&j).unwrap())
            .unwrap_or_default();
        (g, schema)
    } else {
        (ConcurrentGraph::new(), Schema::default())
    };
    let (_, recovery) = DurableWal::open(
        &wal_path(&data),
        snapshot_digest(&data).unwrap(),
        WalMode::Os,
    )
    .unwrap();
    crate::schema::replay_wal(&graph, &mut schema, &recovery.batches);
    TcpServer::with_graph(ServerConfig::default(), graph)
        .with_schema(&schema)
        .unwrap()
}

async fn serve(server: TcpServer) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { server.start_with_listener(listener).await });
    tokio::time::sleep(Duration::from_millis(10)).await;
    addr
}

async fn rows(stream: &mut TcpStream, q: &str) -> Vec<HashMap<String, serde_json::Value>> {
    match query(stream, q).await {
        Response::Result { rows } => rows,
        other => panic!("{q}: {other:?}"),
    }
}
