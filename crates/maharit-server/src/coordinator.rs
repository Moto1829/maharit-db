//! Shard coordinator server.
//!
//! Accepts client connections using the same wire protocol as [`TcpServer`]
//! (4-byte length-prefix + JSON) but instead of executing queries locally it
//! fans them out to the configured shard nodes via [`ShardClient`], merges
//! the results, and returns them to the client.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use maharit_cluster::shard_client::{ShardClient, ShardConnectOptions};
use maharit_cluster::{ClusterConfig, Row, ShardId};
use maharit_query::{Parser, is_read_only};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::auth::{AuthManager, Operation};
use crate::coordinator_plan::{self, CoordPlan, WireRow};
use crate::tls::{DEFAULT_TLS_RELOAD_INTERVAL, ReloadableTlsAcceptor};

/// メッセージ長プレフィックスの上限（バイト）。巨大な長さ宣言による
/// メモリ枯渇 DoS を防ぐ。`tcp_server::MAX_MESSAGE_SIZE` と同値。
const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024; // 64 MiB

// ─── Wire types (mirrors tcp_server) ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum CoordRequest {
    #[serde(rename = "login")]
    Login { username: String, password: String },
    #[serde(rename = "query")]
    Query {
        query: String,
        #[serde(rename = "txId")]
        #[allow(dead_code)]
        tx_id: Option<u64>,
        #[serde(rename = "sessionToken", default)]
        session_token: Option<String>,
    },
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "stats")]
    Stats,
    #[serde(rename = "disconnect")]
    Disconnect,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum CoordResponse {
    #[serde(rename = "result")]
    Result {
        rows: Vec<HashMap<String, serde_json::Value>>,
    },
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(rename = "pong")]
    Pong,
    #[serde(rename = "loggedIn")]
    LoggedIn {
        #[serde(rename = "sessionToken")]
        session_token: String,
        role: String,
    },
    #[serde(rename = "authError")]
    AuthError { message: String },
    #[serde(rename = "stats")]
    Stats {
        connections: u64,
        total_queries: u64,
        nodes: usize,
        edges: usize,
        shards: usize,
    },
    #[serde(rename = "goodbye")]
    Goodbye,
}

// ─── ShardPool ────────────────────────────────────────────────────────────────

/// A pool of [`ShardClient`]s keyed by shard ID, protected by a `Mutex`.
type ShardPool = Arc<HashMap<ShardId, Mutex<ShardClient>>>;

fn build_shard_pool(config: &ClusterConfig, options: &ShardConnectOptions) -> ShardPool {
    let mut map = HashMap::new();
    for shard in &config.shards {
        let client = ShardClient::new(&shard.address).with_options(options.clone());
        map.insert(shard.id, Mutex::new(client));
    }
    Arc::new(map)
}

// ─── ShardCoordinatorServer ───────────────────────────────────────────────────

/// Configuration for the coordinator server.
#[derive(Debug, Clone)]
pub struct CoordinatorConfig {
    /// Address to listen on (e.g. `"127.0.0.1:7690"`).
    pub bind_address: String,
    /// Maximum concurrent client connections.
    pub max_connections: usize,
    /// Per-connection read/write timeout.
    pub timeout: Duration,
    /// Require a valid session token for all requests (except `login`/`ping`).
    /// Defaults to `false` for backward compatibility.
    pub require_auth: bool,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:7690".to_string(),
            max_connections: 100,
            timeout: Duration::from_secs(30),
            require_auth: false,
        }
    }
}

/// Coordinator server that fans queries out to shard nodes and merges results.
pub struct ShardCoordinatorServer {
    coord_config: CoordinatorConfig,
    pool: ShardPool,
    shard_ids: Vec<ShardId>,
    shutdown: Arc<AtomicBool>,
    /// Authentication manager. Only enforced when `coord_config.require_auth = true`.
    auth: Arc<Mutex<AuthManager>>,
    /// TLS for client connections. `None` serves plaintext.
    tls: Option<ReloadableTlsAcceptor>,
    /// Shards, kept to rebuild the pool with other connect options.
    cluster_config: ClusterConfig,
}

impl ShardCoordinatorServer {
    /// Create a new coordinator server from the given configurations.
    pub fn new(coord_config: CoordinatorConfig, cluster_config: ClusterConfig) -> Self {
        let shard_ids: Vec<ShardId> = cluster_config.shards.iter().map(|s| s.id).collect();
        let pool = build_shard_pool(&cluster_config, &ShardConnectOptions::default());
        Self {
            coord_config,
            pool,
            shard_ids,
            shutdown: Arc::new(AtomicBool::new(false)),
            auth: Arc::new(Mutex::new(AuthManager::new())),
            tls: None,
            cluster_config,
        }
    }

    /// Serve client connections over TLS.
    pub fn with_tls(mut self, tls: ReloadableTlsAcceptor) -> Self {
        self.tls = Some(tls);
        self
    }

    /// Connect to the shards over TLS and/or with credentials.
    pub fn with_shard_options(mut self, options: ShardConnectOptions) -> Self {
        self.pool = build_shard_pool(&self.cluster_config, &options);
        self
    }

    /// Replace the authentication manager (e.g. to set a non-default admin
    /// password before enabling `require_auth`).
    pub fn with_auth(mut self, auth: AuthManager) -> Self {
        self.auth = Arc::new(Mutex::new(auth));
        self
    }

    /// Signal the server to stop accepting new connections.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    /// Start listening and serving client connections.
    pub async fn start(self: Arc<Self>) -> std::io::Result<()> {
        let listener = TcpListener::bind(&self.coord_config.bind_address).await?;
        println!(
            "Coordinator listening on {} ({} shards, {})",
            self.coord_config.bind_address,
            self.shard_ids.len(),
            if self.tls.is_some() {
                "TLS"
            } else {
                "plaintext"
            }
        );
        if self.tls.is_none()
            && !crate::tcp_server::is_loopback_bind(&self.coord_config.bind_address)
        {
            eprintln!(
                "WARN: client connections are NOT encrypted (no --tls-cert/--tls-key) and the coordinator listens on {}.",
                self.coord_config.bind_address
            );
        }
        if let Some(tls) = &self.tls {
            tls.spawn_watcher(DEFAULT_TLS_RELOAD_INTERVAL, Arc::clone(&self.shutdown));
        }

        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                break;
            }

            let (stream, peer) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    if self.shutdown.load(Ordering::Relaxed) {
                        break;
                    }
                    eprintln!("Coordinator: accept error: {}", e);
                    continue;
                }
            };

            let srv = Arc::clone(&self);
            let acceptor = self.tls.as_ref().map(ReloadableTlsAcceptor::acceptor);
            tokio::spawn(async move {
                let result = match acceptor {
                    None => srv.handle_connection(stream).await,
                    Some(acceptor) => {
                        match timeout(srv.coord_config.timeout, acceptor.accept(stream)).await {
                            Ok(Ok(tls)) => srv.handle_connection(tls).await,
                            Ok(Err(e)) => Err(std::io::Error::new(
                                e.kind(),
                                format!("TLS handshake failed: {e}"),
                            )),
                            Err(_) => Err(std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                "TLS handshake timed out",
                            )),
                        }
                    }
                };
                if let Err(e) = result {
                    eprintln!("Coordinator: connection from {} error: {}", peer, e);
                }
            });
        }
        Ok(())
    }

    // ── Connection handler ────────────────────────────────────────────────────

    async fn handle_connection<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        mut stream: S,
    ) -> std::io::Result<()> {
        loop {
            // Read request
            let req = match self.read_request(&mut stream).await {
                Ok(r) => r,
                Err(_) => break,
            };

            let resp = match req {
                CoordRequest::Ping => CoordResponse::Pong,
                CoordRequest::Login { username, password } => {
                    let mut mgr = self.auth.lock().await;
                    match mgr.authenticate(&username, &password) {
                        Ok(token) => {
                            let role = mgr
                                .validate_session(&token)
                                .map(|s| role_label(s.role))
                                .unwrap_or_else(|_| "unknown".to_string());
                            CoordResponse::LoggedIn {
                                session_token: token,
                                role,
                            }
                        }
                        Err(e) => CoordResponse::AuthError {
                            message: format!("{}", e),
                        },
                    }
                }
                CoordRequest::Stats => CoordResponse::Stats {
                    connections: 0,
                    total_queries: 0,
                    nodes: 0,
                    edges: 0,
                    shards: self.shard_ids.len(),
                },
                CoordRequest::Disconnect | CoordRequest::Unknown => {
                    let _ = self
                        .write_response(&mut stream, &CoordResponse::Goodbye)
                        .await;
                    break;
                }
                CoordRequest::Query {
                    query,
                    session_token,
                    ..
                } => match self.check_query_auth(&session_token, &query).await {
                    Err(resp) => resp,
                    Ok(()) => match self.execute(&query).await {
                        Ok(rows) => CoordResponse::Result { rows },
                        Err(e) => CoordResponse::Error { message: e },
                    },
                },
            };

            if self.write_response(&mut stream, &resp).await.is_err() {
                break;
            }
        }
        Ok(())
    }

    // ── Authorization ─────────────────────────────────────────────────────────

    /// Validate the session token (when `require_auth`) and enforce RBAC on the
    /// query.  Returns `Ok(())` if the request may proceed, or an `AuthError`
    /// response otherwise.
    async fn check_query_auth(
        &self,
        session_token: &Option<String>,
        query: &str,
    ) -> Result<(), CoordResponse> {
        if !self.coord_config.require_auth {
            return Ok(());
        }

        let token = match session_token {
            Some(t) if !t.is_empty() => t.as_str(),
            _ => {
                return Err(CoordResponse::AuthError {
                    message: "authentication required: missing sessionToken".to_string(),
                });
            }
        };

        let role = {
            let mut mgr = self.auth.lock().await;
            match mgr.validate_session(token) {
                Ok(session) => session.role,
                Err(e) => {
                    return Err(CoordResponse::AuthError {
                        message: format!("invalid session: {}", e),
                    });
                }
            }
        };

        // RBAC: 書き込みクエリは Write 権限を要求する。パース不能なクエリは
        // 権限判定をスキップし、シャード側でエラーを返させる。
        let stmt = match Parser::new(query) {
            Ok(mut parser) => match parser.parse() {
                Ok(s) => s,
                Err(_) => return Ok(()),
            },
            Err(_) => return Ok(()),
        };
        let operation = if is_read_only(&stmt) {
            Operation::Read
        } else {
            Operation::Write
        };

        match AuthManager::check_role_permission(role, operation) {
            Ok(()) => Ok(()),
            Err(e) => Err(CoordResponse::AuthError {
                message: format!("permission denied: {}", e),
            }),
        }
    }

    // ── Query fan-out ─────────────────────────────────────────────────────────

    /// Run a client query: decide how it maps onto the shards
    /// ([`coordinator_plan::plan`]), run it and merge the shard results.
    async fn execute(&self, query: &str) -> Result<Vec<WireRow>, String> {
        match coordinator_plan::plan(query)? {
            CoordPlan::SingleShard => {
                let first = self.shard_ids.iter().take(1).copied().collect::<Vec<_>>();
                let mut batches = self.execute_on_shards(&first, query).await?;
                Ok(batches.pop().unwrap_or_default())
            }
            CoordPlan::FanOut { shard_query, merge } => {
                let batches = self
                    .execute_on_shards(&self.shard_ids, &shard_query)
                    .await?;
                Ok(coordinator_plan::merge_rows(&merge, batches))
            }
        }
    }

    /// Send `query` to the given shards concurrently; one batch per shard.
    async fn execute_on_shards(
        &self,
        shard_ids: &[ShardId],
        query: &str,
    ) -> Result<Vec<Vec<WireRow>>, String> {
        let mut handles = Vec::new();

        for &shard_id in shard_ids {
            if self.pool.contains_key(&shard_id) {
                let q = query.to_string();
                // Clone the Arc so the spawned task owns it.
                let pool = Arc::clone(&self.pool);
                handles.push(tokio::spawn(async move {
                    let mut guard = pool
                        .get(&shard_id)
                        .expect("shard_id must exist in pool")
                        .lock()
                        .await;
                    let addr = guard.addr().to_string();
                    guard
                        .execute(&q)
                        .await
                        .map_err(|e| format!("shard {shard_id} ({addr}): {e}"))
                }));
            }
        }

        // A failed shard fails the whole query: returning the other shards'
        // rows would silently drop data.
        let mut all_rows: Vec<Vec<WireRow>> = Vec::new();
        let mut errors = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(Ok(rows)) => all_rows.push(rows_to_wire(rows)),
                Ok(Err(e)) => errors.push(e),
                Err(e) => errors.push(format!("task join error: {e}")),
            }
        }
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }

        Ok(all_rows)
    }

    // ── Wire helpers ──────────────────────────────────────────────────────────

    async fn read_request<S: AsyncRead + Unpin>(
        &self,
        stream: &mut S,
    ) -> std::io::Result<CoordRequest> {
        let mut len_buf = [0u8; 4];
        timeout(self.coord_config.timeout, stream.read_exact(&mut len_buf))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "read timeout"))??;

        let msg_len = u32::from_be_bytes(len_buf) as usize;
        if msg_len > MAX_MESSAGE_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "message length {} exceeds maximum {}",
                    msg_len, MAX_MESSAGE_SIZE
                ),
            ));
        }
        let mut buf = vec![0u8; msg_len];
        timeout(self.coord_config.timeout, stream.read_exact(&mut buf))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "read timeout"))??;

        serde_json::from_slice(&buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    async fn write_response<S: AsyncWrite + Unpin>(
        &self,
        stream: &mut S,
        resp: &CoordResponse,
    ) -> std::io::Result<()> {
        let payload = serde_json::to_vec(resp)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let len_bytes = (payload.len() as u32).to_be_bytes();

        timeout(self.coord_config.timeout, async {
            stream.write_all(&len_bytes).await?;
            stream.write_all(&payload).await?;
            stream.flush().await
        })
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "write timeout"))?
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Wire label for an authentication role.
fn role_label(role: crate::auth::Role) -> String {
    match role {
        crate::auth::Role::Admin => "admin".to_string(),
        crate::auth::Role::ReadWrite => "read_write".to_string(),
        crate::auth::Role::ReadOnly => "read_only".to_string(),
    }
}

/// Rows as the client protocol sends them: values keep their JSON types.
fn rows_to_wire(rows: Vec<Row>) -> Vec<WireRow> {
    rows.into_iter()
        .map(|row| {
            row.columns
                .into_iter()
                .map(|(k, v)| (k, v.to_json()))
                .collect()
        })
        .collect()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use maharit_cluster::{ClusterConfig, RowValue, ShardConfig};

    fn make_config(n_shards: usize) -> (CoordinatorConfig, ClusterConfig) {
        let shards = (0..n_shards as u32)
            .map(|i| ShardConfig {
                id: i,
                address: format!("127.0.0.1:{}", 7000 + i),
            })
            .collect();
        let cc = CoordinatorConfig::default();
        let cluster = ClusterConfig {
            enabled: true,
            strategy: "hash".into(),
            shards,
            replication_factor: 1,
        };
        (cc, cluster)
    }

    #[test]
    fn test_role_label() {
        use crate::auth::Role;
        assert_eq!(role_label(Role::Admin), "admin");
        assert_eq!(role_label(Role::ReadWrite), "read_write");
        assert_eq!(role_label(Role::ReadOnly), "read_only");
    }

    #[tokio::test]
    async fn test_check_query_auth_disabled_allows_all() {
        let (mut cc, cluster) = make_config(1);
        cc.require_auth = false;
        let srv = ShardCoordinatorServer::new(cc, cluster);
        assert!(srv.check_query_auth(&None, "CREATE (n:X)").await.is_ok());
    }

    #[tokio::test]
    async fn test_check_query_auth_missing_token() {
        let (mut cc, cluster) = make_config(1);
        cc.require_auth = true;
        let srv = ShardCoordinatorServer::new(cc, cluster);
        assert!(
            srv.check_query_auth(&None, "MATCH (n) RETURN n")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_check_query_auth_readonly_denies_write() {
        let (mut cc, cluster) = make_config(1);
        cc.require_auth = true;
        let mut mgr = AuthManager::new();
        mgr.create_user("ro", "pw", crate::auth::Role::ReadOnly)
            .unwrap();
        let srv = ShardCoordinatorServer::new(cc, cluster).with_auth(mgr);
        let token = {
            let mut m = srv.auth.lock().await;
            m.authenticate("ro", "pw").unwrap()
        };
        // 読み取りは許可、書き込みは拒否される。
        assert!(
            srv.check_query_auth(&Some(token.clone()), "MATCH (n) RETURN n")
                .await
                .is_ok()
        );
        assert!(
            srv.check_query_auth(&Some(token), "CREATE (n:X)")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_check_query_auth_admin_allows_write() {
        let (mut cc, cluster) = make_config(1);
        cc.require_auth = true;
        let srv = ShardCoordinatorServer::new(cc, cluster); // 既定 admin/admin
        let token = {
            let mut m = srv.auth.lock().await;
            m.authenticate("admin", "admin").unwrap()
        };
        assert!(
            srv.check_query_auth(&Some(token), "CREATE (n:X)")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_check_query_auth_invalid_token() {
        let (mut cc, cluster) = make_config(1);
        cc.require_auth = true;
        let srv = ShardCoordinatorServer::new(cc, cluster);
        assert!(
            srv.check_query_auth(&Some("bogus".to_string()), "MATCH (n) RETURN n")
                .await
                .is_err()
        );
    }

    #[test]
    fn test_build_shard_pool() {
        let (_, cluster) = make_config(3);
        let pool = build_shard_pool(&cluster, &ShardConnectOptions::default());
        assert_eq!(pool.len(), 3);
        assert!(pool.contains_key(&0));
        assert!(pool.contains_key(&2));
    }

    #[test]
    fn test_coordinator_server_new() {
        let (cc, cluster) = make_config(2);
        let srv = ShardCoordinatorServer::new(cc, cluster);
        assert_eq!(srv.shard_ids.len(), 2);
    }

    #[test]
    fn test_rows_to_wire_conversion() {
        let mut r = Row::new();
        r.insert("name", RowValue::Text("Alice".into()));
        r.insert("age", RowValue::Int(30));
        let wire = rows_to_wire(vec![r]);
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["name"], "Alice");
        assert_eq!(wire[0]["age"], 30);
    }

    // ── Against real shard servers ──────────────────────────────────────────

    async fn start_shard() -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = crate::tcp_server::TcpServer::new(crate::tcp_server::ServerConfig {
            bind_address: addr.to_string(),
            ..Default::default()
        });
        tokio::spawn(async move { server.start_with_listener(listener).await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        addr
    }

    fn coordinator_for(addrs: &[String]) -> ShardCoordinatorServer {
        let cluster = ClusterConfig {
            enabled: true,
            strategy: "hash".into(),
            shards: addrs
                .iter()
                .enumerate()
                .map(|(i, a)| ShardConfig {
                    id: i as u32,
                    address: a.clone(),
                })
                .collect(),
            replication_factor: 1,
        };
        ShardCoordinatorServer::new(CoordinatorConfig::default(), cluster)
    }

    async fn wire(
        srv: &ShardCoordinatorServer,
        q: &str,
    ) -> Result<Vec<HashMap<String, serde_json::Value>>, String> {
        srv.execute(q).await
    }

    #[tokio::test]
    async fn typed_values_and_errors_pass_through_the_coordinator() {
        let a = start_shard().await.to_string();
        let b = start_shard().await.to_string();
        let srv = coordinator_for(&[a.clone(), b.clone()]);
        for (addr, n) in [(&a, 1), (&b, 2)] {
            let mut c = ShardClient::new(addr.as_str());
            c.execute(&format!("CREATE (:P {{n: {n}, f: {n}.5, ok: true}})"))
                .await
                .unwrap();
        }

        let mut rows = wire(
            &srv,
            // Lists and maps cannot be stored as properties yet (graph/146),
            // so build them in RETURN.
            "MATCH (p:P) RETURN p.n AS n, p.f AS f, p.ok AS ok, ['x', 'y'] AS tags, {k: p.n} AS m",
        )
        .await
        .unwrap();
        rows.sort_by_key(|r| r["n"].as_i64());
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0]["n"], 1);
        assert_eq!(rows[1]["f"], 2.5);
        assert_eq!(rows[0]["ok"], true);
        assert_eq!(rows[0]["tags"], serde_json::json!(["x", "y"]));
        assert_eq!(rows[1]["m"], serde_json::json!({"k": 2}));

        // A query error on the shards is reported, not turned into no rows.
        let err = wire(&srv, "MATCH (p:P RETURN p").await.unwrap_err();
        assert!(err.contains("shard 0") && err.contains("shard 1"), "{err}");

        // An unreachable shard fails the query instead of dropping its rows.
        let srv = coordinator_for(&[a, "127.0.0.1:1".to_string()]);
        let err = wire(&srv, "MATCH (p:P) RETURN p.n AS n").await.unwrap_err();
        assert!(err.contains("shard 1 (127.0.0.1:1)"), "{err}");
    }

    /// task154: aggregates, ORDER BY / SKIP / LIMIT and duplicates behave as
    /// on a single database holding the union of the shards; writes are refused.
    #[tokio::test]
    async fn reads_merge_like_a_single_database_and_writes_are_refused() {
        let a = start_shard().await.to_string();
        let b = start_shard().await.to_string();
        let srv = coordinator_for(&[a.clone(), b.clone()]);
        for (addr, names) in [(&a, ["a", "c", "e"]), (&b, ["b", "c", "d"])] {
            let mut c = ShardClient::new(addr.as_str());
            for (i, n) in names.iter().enumerate() {
                c.execute(&format!("CREATE (:P {{name: '{n}', v: {i}}})"))
                    .await
                    .unwrap();
            }
        }

        let rows = wire(&srv, "MATCH (p:P) RETURN count(p) AS c")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["c"], 6);

        let rows = wire(
            &srv,
            "MATCH (p:P) RETURN p.name AS name, count(p) AS c, sum(p.v) AS s ORDER BY c DESC, name LIMIT 2",
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "c");
        assert_eq!(rows[0]["c"], 2);
        assert_eq!(rows[0]["s"], 2);
        assert_eq!(rows[1]["name"], "a");

        let rows = wire(
            &srv,
            "MATCH (p:P) RETURN p.name AS name ORDER BY name SKIP 1 LIMIT 3",
        )
        .await
        .unwrap();
        let names: Vec<&str> = rows.iter().map(|r| r["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["b", "c", "c"]);

        let rows = wire(&srv, "MATCH (p:P) RETURN DISTINCT p.name AS name")
            .await
            .unwrap();
        assert_eq!(rows.len(), 5);
        let rows = wire(&srv, "RETURN 1 AS one").await.unwrap();
        assert_eq!(rows.len(), 1);

        let err = wire(&srv, "CREATE (:P {name: 'x'})").await.unwrap_err();
        assert!(err.contains("read-only"), "{err}");
        let err = wire(&srv, "MATCH (p:P) RETURN avg(p.v)").await.unwrap_err();
        assert!(err.contains("not supported"), "{err}");
        let rows = wire(&srv, "MATCH (p:P {name: 'x'}) RETURN p")
            .await
            .unwrap();
        assert!(rows.is_empty());
    }

    // ── TLS and shard authentication (task151) ──────────────────────────────

    /// A shard serving TLS with the test certificate and requiring login as
    /// admin/admin, with sessions that expire after `session`.
    async fn start_secure_shard(cert: &str, key: &str, session: Duration) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let tls = ReloadableTlsAcceptor::new(crate::tls::TlsConfig::new(cert, key)).unwrap();
        let server = crate::tcp_server::TcpServer::new(crate::tcp_server::ServerConfig {
            bind_address: addr.to_string(),
            require_auth: true,
            ..Default::default()
        })
        .with_auth(AuthManager::with_session_timeout(session))
        .with_tls(tls);
        tokio::spawn(async move { server.start_with_listener(listener).await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        addr.to_string()
    }

    #[tokio::test]
    async fn coordinator_reaches_tls_shards_with_credentials() {
        use crate::tls::test_support::*;
        let cert = TempPem::new("shard_cert", TEST_SERVER_CERT_PEM);
        let key = TempPem::new("shard_key", TEST_SERVER_KEY_PEM);
        let other = TempPem::new("shard_other", TEST_SERVER_CERT2_PEM);
        let shard = start_secure_shard(cert.path(), key.path(), Duration::from_secs(1)).await;
        let options = |ca: &str, password: &str| ShardConnectOptions {
            tls: Some(crate::tls::build_client_config(Some(ca), false).unwrap()),
            credentials: Some(("admin".to_string(), password.to_string())),
        };

        let srv = coordinator_for(std::slice::from_ref(&shard))
            .with_shard_options(options(cert.path(), "admin"));
        // The coordinator is read-only: write to the shard directly.
        ShardClient::new(shard.as_str())
            .with_options(options(cert.path(), "admin"))
            .execute("CREATE (:S {v: 1})")
            .await
            .unwrap();
        let rows = wire(&srv, "MATCH (s:S) RETURN s.v AS v").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["v"], 1);

        // The session expires: the client logs in again and the query works.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let rows = wire(&srv, "MATCH (s:S) RETURN s.v AS v").await.unwrap();
        assert_eq!(rows.len(), 1, "after session expiry");

        // Wrong password, untrusted certificate, and plaintext all fail.
        let bad_pw = coordinator_for(std::slice::from_ref(&shard))
            .with_shard_options(options(cert.path(), "wrong"));
        let err = wire(&bad_pw, "MATCH (s:S) RETURN s").await.unwrap_err();
        assert!(err.contains("credentials"), "{err}");
        let untrusted = coordinator_for(std::slice::from_ref(&shard))
            .with_shard_options(options(other.path(), "admin"));
        let err = wire(&untrusted, "MATCH (s:S) RETURN s").await.unwrap_err();
        assert!(err.contains("TLS"), "{err}");
        let plain = coordinator_for(std::slice::from_ref(&shard));
        assert!(wire(&plain, "MATCH (s:S) RETURN s").await.is_err());
    }

    #[tokio::test]
    async fn coordinator_serves_clients_over_tls() {
        use crate::tls::test_support::*;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let cert = TempPem::new("coord_cert", TEST_SERVER_CERT_PEM);
        let key = TempPem::new("coord_key", TEST_SERVER_KEY_PEM);
        let shard = start_shard().await.to_string();

        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);
        let cluster = ClusterConfig {
            enabled: true,
            strategy: "hash".into(),
            shards: vec![ShardConfig {
                id: 0,
                address: shard,
            }],
            replication_factor: 1,
        };
        let tls = ReloadableTlsAcceptor::new(crate::tls::TlsConfig::new(cert.path(), key.path()))
            .unwrap();
        let srv = Arc::new(
            ShardCoordinatorServer::new(
                CoordinatorConfig {
                    bind_address: addr.to_string(),
                    ..Default::default()
                },
                cluster,
            )
            .with_tls(tls),
        );
        tokio::spawn(Arc::clone(&srv).start());
        tokio::time::sleep(Duration::from_millis(50)).await;

        let config = crate::tls::build_client_config(Some(cert.path()), false).unwrap();
        let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        let name = rustls::pki_types::ServerName::try_from("127.0.0.1").unwrap();
        let mut stream = tokio_rustls::TlsConnector::from(config)
            .connect(name, tcp)
            .await
            .unwrap();
        let body = br#"{"type":"query","query":"RETURN 1 AS one"}"#;
        stream
            .write_all(&(body.len() as u32).to_be_bytes())
            .await
            .unwrap();
        stream.write_all(body).await.unwrap();
        stream.flush().await.unwrap();
        let mut len = [0u8; 4];
        stream.read_exact(&mut len).await.unwrap();
        let mut buf = vec![0u8; u32::from_be_bytes(len) as usize];
        stream.read_exact(&mut buf).await.unwrap();
        let resp: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(resp["type"], "result", "{resp}");
        assert_eq!(resp["rows"][0]["one"], 1);

        // A plaintext client gets no answer.
        let mut plain = tokio::net::TcpStream::connect(addr).await.unwrap();
        plain
            .write_all(&(body.len() as u32).to_be_bytes())
            .await
            .unwrap();
        plain.write_all(body).await.unwrap();
        let mut out = Vec::new();
        let _ = timeout(Duration::from_secs(5), plain.read_to_end(&mut out)).await;
        assert!(
            !out.windows(8).any(|w| w == b"\"result\""),
            "plaintext answered"
        );
        srv.shutdown();
    }
}
