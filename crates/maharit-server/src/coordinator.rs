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

use maharit_cluster::shard_client::ShardClient;
use maharit_cluster::{ClusterConfig, Row, RowValue, ShardId};
use maharit_query::{Parser, is_read_only};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::auth::{AuthManager, Operation};

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
    Result { rows: Vec<HashMap<String, String>> },
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

fn build_shard_pool(config: &ClusterConfig) -> ShardPool {
    let mut map = HashMap::new();
    for shard in &config.shards {
        map.insert(shard.id, Mutex::new(ShardClient::new(&shard.address)));
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
}

impl ShardCoordinatorServer {
    /// Create a new coordinator server from the given configurations.
    pub fn new(coord_config: CoordinatorConfig, cluster_config: ClusterConfig) -> Self {
        let shard_ids: Vec<ShardId> = cluster_config.shards.iter().map(|s| s.id).collect();
        let pool = build_shard_pool(&cluster_config);
        Self {
            coord_config,
            pool,
            shard_ids,
            shutdown: Arc::new(AtomicBool::new(false)),
            auth: Arc::new(Mutex::new(AuthManager::new())),
        }
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
            "Coordinator listening on {} ({} shards)",
            self.coord_config.bind_address,
            self.shard_ids.len()
        );

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
            tokio::spawn(async move {
                if let Err(e) = srv.handle_connection(stream).await {
                    eprintln!("Coordinator: connection from {} error: {}", peer, e);
                }
            });
        }
        Ok(())
    }

    // ── Connection handler ────────────────────────────────────────────────────

    async fn handle_connection(&self, mut stream: TcpStream) -> std::io::Result<()> {
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
                    let _ = self.write_response(&mut stream, &CoordResponse::Goodbye).await;
                    break;
                }
                CoordRequest::Query {
                    query,
                    session_token,
                    ..
                } => match self.check_query_auth(&session_token, &query).await {
                    Err(resp) => resp,
                    Ok(()) => match self.execute_on_all_shards(&query).await {
                        Ok(rows) => CoordResponse::Result {
                            rows: rows_to_wire(rows),
                        },
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

    /// Send `query` to every known shard concurrently and merge results.
    async fn execute_on_all_shards(&self, query: &str) -> Result<Vec<Row>, String> {
        let mut handles = Vec::new();

        for &shard_id in &self.shard_ids {
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
                    guard.execute(&q).await.unwrap_or_default()
                }));
            }
        }

        let mut all_rows: Vec<Vec<Row>> = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(rows) => all_rows.push(rows),
                Err(e) => return Err(format!("Task join error: {}", e)),
            }
        }

        Ok(merge_rows(all_rows))
    }

    // ── Wire helpers ──────────────────────────────────────────────────────────

    async fn read_request(&self, stream: &mut TcpStream) -> std::io::Result<CoordRequest> {
        let mut len_buf = [0u8; 4];
        timeout(self.coord_config.timeout, stream.read_exact(&mut len_buf))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "read timeout"))??;

        let msg_len = u32::from_be_bytes(len_buf) as usize;
        if msg_len > MAX_MESSAGE_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("message length {} exceeds maximum {}", msg_len, MAX_MESSAGE_SIZE),
            ));
        }
        let mut buf = vec![0u8; msg_len];
        timeout(self.coord_config.timeout, stream.read_exact(&mut buf))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "read timeout"))??;

        serde_json::from_slice(&buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    async fn write_response(
        &self,
        stream: &mut TcpStream,
        resp: &CoordResponse,
    ) -> std::io::Result<()> {
        let payload = serde_json::to_vec(resp)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let len_bytes = (payload.len() as u32).to_be_bytes();

        timeout(self.coord_config.timeout, async {
            stream.write_all(&len_bytes).await?;
            stream.write_all(&payload).await
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

/// Deduplicate and merge rows from multiple shards.
fn merge_rows(batches: Vec<Vec<Row>>) -> Vec<Row> {
    use std::collections::HashSet;

    let mut seen: HashSet<Vec<(String, String)>> = HashSet::new();
    let mut out = Vec::new();

    for batch in batches {
        for row in batch {
            let mut key: Vec<(String, String)> = row
                .columns
                .iter()
                .map(|(k, v)| (k.clone(), row_value_to_string(v)))
                .collect();
            key.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
            if seen.insert(key) {
                out.push(row);
            }
        }
    }
    out
}

fn row_value_to_string(v: &RowValue) -> String {
    match v {
        RowValue::Null => "null".to_string(),
        RowValue::Int(n) => n.to_string(),
        RowValue::Text(s) => s.clone(),
        RowValue::Bool(b) => b.to_string(),
    }
}

fn rows_to_wire(rows: Vec<Row>) -> Vec<HashMap<String, String>> {
    rows.into_iter()
        .map(|row| {
            row.columns
                .into_iter()
                .map(|(k, v)| (k, row_value_to_string(&v)))
                .collect()
        })
        .collect()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use maharit_cluster::{ClusterConfig, ShardConfig};

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
        let pool = build_shard_pool(&cluster);
        assert_eq!(pool.len(), 3);
        assert!(pool.contains_key(&0));
        assert!(pool.contains_key(&2));
    }

    #[test]
    fn test_merge_rows_dedup() {
        let mut r = Row::new();
        r.insert("x", RowValue::Int(1));
        let batches = vec![vec![r.clone()], vec![r]];
        let merged = merge_rows(batches);
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn test_merge_rows_distinct() {
        let mut r1 = Row::new();
        r1.insert("x", RowValue::Int(1));
        let mut r2 = Row::new();
        r2.insert("x", RowValue::Int(2));
        let merged = merge_rows(vec![vec![r1], vec![r2]]);
        assert_eq!(merged.len(), 2);
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
        assert_eq!(wire[0].get("name").unwrap(), "Alice");
        assert_eq!(wire[0].get("age").unwrap(), "30");
    }
}
