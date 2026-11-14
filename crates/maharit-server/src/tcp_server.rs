//! TCP server for network-based query execution
//!
//! Provides:
//! - TCP connection handling with async I/O
//! - Length-prefixed message framing
//! - Optional TLS (with certificate hot reload)
//! - JSON request/response protocol
//! - Connection pool management
//! - Graceful shutdown

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use bytes::{Buf, BytesMut};
use maharit_core::{ConcurrentGraph, ConstraintManager, FulltextManager, PropertyIndex};
use maharit_query::{AstCache, Executor, Parser, Value, is_read_only};
use maharit_storage::TransactionManager;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::time::timeout;
use tracing::Instrument;

use crate::durable_wal::{DurableWal, snapshot_digest};
use crate::mutation_log::RecordingGraph;
use crate::replication::{
    FollowerReplicationManager, LeaderReplicationManager, ReplicaApply, ReplicationConfig,
    ReplicationStats, SnapshotSource, WalEntryData,
};
use crate::schema::Schema;
use crate::tls::{DEFAULT_TLS_RELOAD_INTERVAL, ReloadableTlsAcceptor};
use crate::tracing_setup::TracingConfig;
use maharit_storage::UndoRecord;

/// Server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Address to bind to
    pub bind_address: String,
    /// Maximum number of concurrent connections
    pub max_connections: usize,
    /// Read timeout for client connections
    pub read_timeout: Duration,
    /// Write timeout for client connections
    pub write_timeout: Duration,
    /// 認証を必須にするかどうか。デフォルトは `false`（互換性のため）。
    /// `true` のとき、`Login` 以外のリクエストは有効な `sessionToken` を要求する。
    pub require_auth: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:7687".to_string(),
            max_connections: 100,
            read_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(30),
            require_auth: false,
        }
    }
}

/// Default chunk size for streaming results
pub const DEFAULT_CHUNK_SIZE: usize = 100;

/// メッセージ長プレフィックスの上限（バイト）。
/// クライアントが宣言する長さがこれを超える場合は接続を切断する。
/// 悪意あるクライアントが巨大な長さを宣言してサーバーのメモリを枯渇させる
/// DoS を防ぐためのガード。
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024; // 64 MiB

/// セッションタイムアウト（秒）。`AuthManager` の `session_timeout` (30 分) と
/// 揃える。クライアントに返す expires_at の算出に使う。
pub const DEFAULT_SESSION_TIMEOUT_SECS: u64 = 30 * 60;

/// 認証ロールをワイヤ表現の文字列にする。
fn role_label(role: &crate::auth::Role) -> String {
    match role {
        crate::auth::Role::Admin => "admin".to_string(),
        crate::auth::Role::ReadWrite => "read_write".to_string(),
        crate::auth::Role::ReadOnly => "read_only".to_string(),
    }
}

/// `require_auth = true` のとき、リクエストの `sessionToken` を検証し、
/// 認証済みセッションのロールを返す。
///
/// - 認証が無効なら検証せず `Ok(Role::Admin)`（後段の権限チェックも実質無効化）
/// - トークン無し → `Err(AuthError レスポンス)`
/// - トークンが無効/期限切れ → `Err(AuthError レスポンス)`
/// - 有効 → `Ok(role)`
fn check_session(
    require_auth: bool,
    auth: &Arc<Mutex<crate::auth::AuthManager>>,
    token: &Option<String>,
) -> Result<crate::auth::Role, Response> {
    check_session_user(require_auth, auth, token).map(|(role, _)| role)
}

/// [`check_session`] と同じだが、監査ログ用にユーザー名も返す
/// （認証が無効なら `"anonymous"`）。
fn check_session_user(
    require_auth: bool,
    auth: &Arc<Mutex<crate::auth::AuthManager>>,
    token: &Option<String>,
) -> Result<(crate::auth::Role, String), Response> {
    if !require_auth {
        return Ok((crate::auth::Role::Admin, "anonymous".to_string()));
    }
    let token_str = match token {
        Some(t) if !t.is_empty() => t.as_str(),
        _ => {
            return Err(Response::AuthError {
                message: "authentication required: missing sessionToken".to_string(),
            });
        }
    };
    let mut mgr = auth.lock().unwrap();
    match mgr.validate_session(token_str) {
        Ok(session) => Ok((session.role, session.username.clone())),
        Err(e) => Err(Response::AuthError {
            message: format!("invalid session: {}", e),
        }),
    }
}

/// 認証済みロールが対象クエリを実行できるかを RBAC で検証する。
///
/// 書き込みを伴うクエリ（CREATE/SET/DELETE/MERGE/REMOVE/制約作成など）を
/// `ReadOnly` ロールが実行しようとした場合に拒否レスポンスを返す。
/// パースに失敗するクエリは権限判定をスキップし、実行側でパースエラーを返させる。
fn authorize_query(role: crate::auth::Role, query: &str) -> Option<Response> {
    let stmt = match Parser::new(query) {
        Ok(mut parser) => match parser.parse() {
            Ok(s) => s,
            Err(_) => return None,
        },
        Err(_) => return None,
    };

    let operation = if is_read_only(&stmt) {
        crate::auth::Operation::Read
    } else {
        crate::auth::Operation::Write
    };

    match crate::auth::AuthManager::check_role_permission(role, operation) {
        Ok(()) => None,
        Err(e) => Some(Response::AuthError {
            message: format!("permission denied: {}", e),
        }),
    }
}

/// ロール文字列を [`crate::auth::Role`] に対応付ける。
/// `reader`/`read_only`、`writer`/`read_write`、`admin` を受理（大文字小文字無視）。
fn parse_role_str(s: &str) -> Option<crate::auth::Role> {
    match s.to_ascii_lowercase().as_str() {
        "reader" | "read_only" | "readonly" => Some(crate::auth::Role::ReadOnly),
        "writer" | "read_write" | "readwrite" => Some(crate::auth::Role::ReadWrite),
        "admin" => Some(crate::auth::Role::Admin),
        _ => None,
    }
}

/// 監査ログが有効なら `event` を記録する。
fn audit_log(audit: Option<&crate::audit::AuditLogger>, event: crate::audit::AuthEvent) {
    if let Some(a) = audit {
        a.log_auth_event(event);
    }
}

/// 権限エラーを監査ログに記録する。
fn audit_denied(audit: Option<&crate::audit::AuditLogger>, username: &str, action: &str) {
    audit_log(
        audit,
        crate::audit::AuthEvent::PermissionDenied {
            username: username.to_string(),
            action: action.to_string(),
        },
    );
}

/// 単一メッセージの `result` レスポンスを作る。
fn result_message(msg: String) -> Response {
    let mut row = HashMap::new();
    row.insert("result".to_string(), serde_json::Value::String(msg));
    Response::Result { rows: vec![row] }
}

/// ユーザー管理文（CREATE/DROP/ALTER USER, SHOW USERS）をサーバーの
/// [`AuthManager`] に配線する。対象文なら `Some(Response)` を返し、そうでなければ
/// `None`（呼び出し側は通常のクエリ実行にフォールバック）。
///
/// クエリ実行エンジンはこれらを stub（実際には何もしない）として扱うため、
/// ここでサーバーの認証状態に対して実処理する。認証有効時は admin（ManageUsers）
/// 権限を要求する。
fn try_handle_user_management(
    query: &str,
    auth: &Arc<Mutex<crate::auth::AuthManager>>,
    ast_cache: &Arc<Mutex<AstCache>>,
    role: crate::auth::Role,
    require_auth: bool,
    audit: Option<&crate::audit::AuditLogger>,
    actor: &str,
) -> Option<Response> {
    use crate::audit::AuthEvent;
    use maharit_query::ast::Statement;

    let stmt = ast_cache.lock().unwrap().get_or_parse(query).ok()?;
    let is_user_mgmt = matches!(
        stmt,
        Statement::CreateUser(_)
            | Statement::DropUser(_)
            | Statement::AlterUser(_)
            | Statement::ShowUsers
    );
    if !is_user_mgmt {
        return None;
    }

    // RBAC: 認証有効時はユーザー管理を admin (ManageUsers) に限定する。
    if require_auth
        && crate::auth::AuthManager::check_role_permission(
            role,
            crate::auth::Operation::ManageUsers,
        )
        .is_err()
    {
        audit_denied(audit, actor, "manage_users");
        return Some(Response::AuthError {
            message: "permission denied: user management requires admin role".to_string(),
        });
    }

    let mut mgr = auth.lock().unwrap();
    let resp = match stmt {
        Statement::CreateUser(cu) => match parse_role_str(&cu.role) {
            Some(r) => match mgr.create_user(&cu.username, &cu.password, r) {
                Ok(()) => {
                    audit_log(
                        audit,
                        AuthEvent::UserCreated {
                            username: cu.username.clone(),
                            by: actor.to_string(),
                        },
                    );
                    result_message(format!(
                        "User '{}' created with role '{}'",
                        cu.username, cu.role
                    ))
                }
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            },
            None => Response::Error {
                message: format!("unknown role: '{}'", cu.role),
            },
        },
        Statement::DropUser(du) => match mgr.drop_user(&du.username) {
            Ok(()) => {
                audit_log(
                    audit,
                    AuthEvent::UserDeleted {
                        username: du.username.clone(),
                        by: actor.to_string(),
                    },
                );
                result_message(format!("User '{}' dropped", du.username))
            }
            Err(e) => Response::Error {
                message: e.to_string(),
            },
        },
        Statement::AlterUser(au) => {
            let mut result: Result<(), crate::auth::AuthError> = Ok(());
            if let Some(pw) = &au.password {
                result = mgr.alter_user_password(&au.username, pw);
            }
            if result.is_ok()
                && let Some(rl) = &au.role
            {
                match parse_role_str(rl) {
                    Some(r) => result = mgr.alter_user_role(&au.username, r),
                    None => {
                        return Some(Response::Error {
                            message: format!("unknown role: '{}'", rl),
                        });
                    }
                }
            }
            match result {
                Ok(()) => {
                    audit_log(
                        audit,
                        AuthEvent::UserModified {
                            username: au.username.clone(),
                            by: actor.to_string(),
                        },
                    );
                    result_message(format!("User '{}' altered", au.username))
                }
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
        Statement::ShowUsers => {
            let rows: Vec<HashMap<String, serde_json::Value>> = mgr
                .list_users()
                .iter()
                .map(|u| {
                    let mut m = HashMap::new();
                    m.insert(
                        "username".to_string(),
                        serde_json::Value::String(u.username.clone()),
                    );
                    m.insert(
                        "role".to_string(),
                        serde_json::Value::String(role_label(&u.role)),
                    );
                    m.insert("active".to_string(), serde_json::Value::Bool(u.active));
                    m
                })
                .collect();
            Response::Result { rows }
        }
        _ => unreachable!("filtered to user-management statements above"),
    };
    Some(resp)
}

/// Request message from client
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    /// Authenticate with username/password. Returns a session token to use
    /// in subsequent requests via `sessionToken`. Always permitted regardless
    /// of `ServerConfig::require_auth`.
    #[serde(rename = "login")]
    Login { username: String, password: String },

    /// Execute a query
    #[serde(rename = "query")]
    Query {
        query: String,
        /// Optional query parameters, referenced as `$name` in the query.
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        params: HashMap<String, serde_json::Value>,
        /// Optional transaction ID for executing within a transaction
        #[serde(rename = "txId")]
        tx_id: Option<u64>,
        /// Optional session token (required when server has `require_auth=true`)
        #[serde(rename = "sessionToken", default)]
        session_token: Option<String>,
    },

    /// Execute a query with streaming results
    #[serde(rename = "streamQuery")]
    StreamQuery {
        query: String,
        /// Optional query parameters, referenced as `$name` in the query.
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        params: HashMap<String, serde_json::Value>,
        /// Optional transaction ID for executing within a transaction
        #[serde(rename = "txId")]
        tx_id: Option<u64>,
        /// Number of rows per chunk (default: 100)
        #[serde(rename = "chunkSize", default = "default_chunk_size")]
        chunk_size: usize,
        /// Optional session token (required when server has `require_auth=true`)
        #[serde(rename = "sessionToken", default)]
        session_token: Option<String>,
    },

    /// Ping to check server health
    #[serde(rename = "ping")]
    Ping,

    /// Get server statistics
    #[serde(rename = "stats")]
    Stats,

    /// Disconnect gracefully
    #[serde(rename = "disconnect")]
    Disconnect,

    /// Begin a new transaction
    #[serde(rename = "begin")]
    BeginTransaction {
        /// If true, the transaction is read-only
        #[serde(rename = "readOnly", default)]
        read_only: bool,
        /// Optional session token (required when server has `require_auth=true`)
        #[serde(rename = "sessionToken", default)]
        session_token: Option<String>,
    },

    /// Commit a transaction
    #[serde(rename = "commit")]
    Commit {
        #[serde(rename = "txId")]
        tx_id: u64,
        #[serde(rename = "sessionToken", default)]
        session_token: Option<String>,
    },

    /// Rollback a transaction
    #[serde(rename = "rollback")]
    Rollback {
        #[serde(rename = "txId")]
        tx_id: u64,
        #[serde(rename = "sessionToken", default)]
        session_token: Option<String>,
    },

    /// Promote this follower to leader (admin only).
    #[serde(rename = "promote")]
    Promote {
        #[serde(rename = "sessionToken", default)]
        session_token: Option<String>,
    },
}

/// Replication status embedded in the `stats` response.
#[derive(Debug, Serialize, Deserialize)]
pub struct ReplicationStatus {
    /// `"leader"` or `"follower"`.
    pub role: String,
    /// This node's identifier in the cluster.
    pub node_id: String,
    /// Current log sequence number (applied WAL position).
    pub current_lsn: u64,
    /// Number of connected followers (leader only; 0 for a follower).
    pub follower_count: usize,
    /// For a follower: whether the leader is currently reachable (heartbeats
    /// arriving). For a leader: always `true` (it is the leader).
    #[serde(rename = "is_leader_alive")]
    pub is_leader_alive: bool,
}

impl From<ReplicationStats> for ReplicationStatus {
    fn from(s: ReplicationStats) -> Self {
        Self {
            // ReplicationStats uses "Leader"/"Follower"; expose lowercase on the wire.
            role: s.role.to_lowercase(),
            node_id: s.node_id,
            current_lsn: s.current_lsn,
            follower_count: s.follower_count,
            is_leader_alive: s.is_leader_alive,
        }
    }
}

fn default_chunk_size() -> usize {
    DEFAULT_CHUNK_SIZE
}

/// Response message to client
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    /// Query result
    #[serde(rename = "result")]
    /// クエリ結果。各行は列名 → JSON 値のマップ。
    /// 値は Cypher の型情報を保持する: 文字列は JSON String, 数値は JSON Number,
    /// bool は JSON Boolean, null は JSON Null, リスト/マップは JSON Array/Object。
    Result {
        rows: Vec<HashMap<String, serde_json::Value>>,
    },

    /// Error response
    #[serde(rename = "error")]
    Error { message: String },

    /// Pong response
    #[serde(rename = "pong")]
    Pong,

    /// Statistics response
    #[serde(rename = "stats")]
    Stats {
        connections: u64,
        total_queries: u64,
        nodes: usize,
        edges: usize,
        /// Replication status for leader/follower nodes; absent for a
        /// standalone server.
        #[serde(skip_serializing_if = "Option::is_none")]
        replication: Option<ReplicationStatus>,
    },

    /// Goodbye response before disconnect
    #[serde(rename = "goodbye")]
    Goodbye,

    /// Transaction started successfully
    #[serde(rename = "transactionBegun")]
    TransactionBegun {
        #[serde(rename = "txId")]
        tx_id: u64,
    },

    /// Transaction committed successfully
    #[serde(rename = "committed")]
    Committed {
        #[serde(rename = "txId")]
        tx_id: u64,
    },

    /// Transaction rolled back successfully
    #[serde(rename = "rolledBack")]
    RolledBack {
        #[serde(rename = "txId")]
        tx_id: u64,
    },

    /// Start of streaming response
    #[serde(rename = "streamStart")]
    StreamStart {
        /// Unique stream ID for this stream session
        #[serde(rename = "streamId")]
        stream_id: u64,
        /// Total number of rows (if known)
        #[serde(rename = "totalRows", skip_serializing_if = "Option::is_none")]
        total_rows: Option<usize>,
    },

    /// A chunk of streaming data
    #[serde(rename = "streamChunk")]
    StreamChunk {
        /// Stream ID this chunk belongs to
        #[serde(rename = "streamId")]
        stream_id: u64,
        /// Chunk sequence number (0-indexed)
        #[serde(rename = "chunkIndex")]
        chunk_index: usize,
        /// Rows in this chunk
        rows: Vec<HashMap<String, serde_json::Value>>,
    },

    /// End of streaming response
    #[serde(rename = "streamEnd")]
    StreamEnd {
        /// Stream ID that ended
        #[serde(rename = "streamId")]
        stream_id: u64,
        /// Total rows sent
        #[serde(rename = "totalRows")]
        total_rows: usize,
    },

    /// The follower was promoted and now accepts writes.
    #[serde(rename = "promoted")]
    Promoted {
        /// Address now accepting followers, when the node serves replication.
        #[serde(rename = "replicationBind", skip_serializing_if = "Option::is_none")]
        replication_bind: Option<String>,
    },

    /// Successful login. Subsequent requests can carry `sessionToken`.
    #[serde(rename = "loggedIn")]
    LoggedIn {
        #[serde(rename = "sessionToken")]
        session_token: String,
        /// User role: "admin" / "read_write" / "read_only"
        role: String,
        /// Unix epoch seconds when this session expires
        #[serde(rename = "expiresAt")]
        expires_at: u64,
    },

    /// Authentication error (missing/expired/invalid token, or wrong credentials)
    #[serde(rename = "authError")]
    AuthError { message: String },
}

/// Statistics for the server
#[derive(Debug, Default)]
pub struct ServerStats {
    pub current_connections: AtomicU64,
    pub total_connections: AtomicU64,
    pub total_queries: AtomicU64,
    next_stream_id: AtomicU64,
}

impl ServerStats {
    /// Generate a unique stream ID
    pub fn next_stream_id(&self) -> u64 {
        self.next_stream_id.fetch_add(1, Ordering::SeqCst)
    }
}

/// TCP server for MaharitDB
pub struct TcpServer {
    config: ServerConfig,
    graph: Arc<ConcurrentGraph>,
    stats: Arc<ServerStats>,
    shutdown: Arc<AtomicBool>,
    tx_manager: Arc<TransactionManager>,
    /// Shared constraint / fulltext / property-index managers: persist across
    /// all query executions.
    managers: Arc<SharedManagers>,
    /// Optional leader replication manager: when set, write operations are
    /// automatically replicated to followers via WAL entries.
    replication: SharedLeader,
    /// Optional follower replication manager: when set, the node rejects
    /// writes until it is promoted, and `stats` reports leader liveness.
    follower: Option<Arc<FollowerReplicationManager>>,
    /// How a follower becomes a leader when promoted.
    promotion: Option<Arc<Promotion>>,
    /// Authentication manager. Only enforced when `config.require_auth = true`.
    auth: Arc<Mutex<crate::auth::AuthManager>>,
    /// Shared parsed-AST cache: avoids re-parsing identical query strings on the
    /// hot request path. Keyed by normalized query text.
    ast_cache: Arc<Mutex<AstCache>>,
    /// On-disk WAL + checkpoint settings. `None` keeps the old behaviour of
    /// saving only at shutdown.
    durability: Option<Arc<Durability>>,
    /// TLS for client connections. `None` serves plaintext.
    tls: Option<ReloadableTlsAcceptor>,
    /// Prometheus metrics served by the monitoring HTTP server (`/metrics`).
    metrics: Arc<crate::metrics::Metrics>,
    /// 監査ログ（`--audit-log`）。ログイン・ユーザー管理・権限エラーを記録する。
    audit: Option<Arc<crate::audit::AuditLogger>>,
}

/// The leader replication manager, if this node currently is a leader. Set at
/// startup on a leader, or when a follower is promoted.
pub type SharedLeader = Arc<std::sync::RwLock<Option<Arc<LeaderReplicationManager>>>>;

/// What a follower does to become a leader.
pub struct Promotion {
    /// Leader replication settings (role, bind address, secret, node ID).
    /// `None` makes the promoted node a standalone leader without followers.
    pub replication: Option<ReplicationConfig>,
    /// TLS for the replication listener.
    pub tls: Option<ReloadableTlsAcceptor>,
}

/// On-disk durability for a server: the WAL and the snapshot it applies to.
pub struct Durability {
    wal: Arc<DurableWal>,
    data_path: std::path::PathBuf,
    /// Take a checkpoint (save the snapshot, empty the WAL) once the WAL grows
    /// past this many bytes. 0 disables size-triggered checkpoints.
    checkpoint_bytes: u64,
}

impl Durability {
    pub fn new(
        wal: DurableWal,
        data_path: impl Into<std::path::PathBuf>,
        checkpoint_bytes: u64,
    ) -> Self {
        Self {
            wal: Arc::new(wal),
            data_path: data_path.into(),
            checkpoint_bytes,
        }
    }
}

/// Writes the graph and its schema to a data file.
#[derive(Clone)]
pub struct SnapshotWriter {
    graph: Arc<ConcurrentGraph>,
    managers: Arc<SharedManagers>,
}

impl SnapshotWriter {
    /// Save atomically to `path`, with writers paused.
    pub async fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let _gate = self.managers.write_gate.lock().await;
        save_snapshot(&self.graph, &self.managers, path)
    }
}

/// Save `graph` and the managers' schema to `path` (v5 data file).
fn save_snapshot(
    graph: &ConcurrentGraph,
    managers: &SharedManagers,
    path: &std::path::Path,
) -> std::io::Result<()> {
    let schema = managers.export_schema();
    let schema_json = if schema.is_empty() {
        String::new()
    } else {
        serde_json::to_string(&schema).map_err(std::io::Error::other)?
    };
    maharit_storage::PersistentStorage::save_concurrent_with_schema(graph, &schema_json, path)
        .map_err(|e| std::io::Error::other(e.to_string()))
}

/// Saves a snapshot and empties the WAL, with writers paused.
#[derive(Clone)]
pub struct Checkpointer {
    graph: Arc<ConcurrentGraph>,
    managers: Arc<SharedManagers>,
    durability: Arc<Durability>,
}

impl Checkpointer {
    /// Write the snapshot atomically, then start an empty WAL for it. Holds the
    /// write gate throughout so no write lands between the two steps.
    pub async fn checkpoint(&self) -> std::io::Result<()> {
        let _gate = self.managers.write_gate.lock().await;
        self.checkpoint_gated().await
    }

    /// [`checkpoint`](Self::checkpoint) for a caller already holding the
    /// write gate.
    async fn checkpoint_gated(&self) -> std::io::Result<()> {
        let graph = Arc::clone(&self.graph);
        let managers = Arc::clone(&self.managers);
        let durability = Arc::clone(&self.durability);
        tokio::task::spawn_blocking(move || {
            let path = &durability.data_path;
            save_snapshot(&graph, &managers, path)?;
            durability.wal.reset(snapshot_digest(path)?)
        })
        .await
        .map_err(std::io::Error::other)?
    }
}

/// Where a write statement's mutations go: the on-disk WAL and/or followers.
#[derive(Clone, Copy)]
struct WriteSinks<'a> {
    replication: Option<&'a LeaderReplicationManager>,
    wal: Option<&'a Arc<DurableWal>>,
}

impl WriteSinks<'_> {
    /// Whether executors must record mutations at all.
    fn record(&self) -> bool {
        self.replication.is_some() || self.wal.is_some()
    }

    /// Append the statement's mutations to the WAL and send them to followers.
    /// Call while holding the write gate. Returns the WAL offset to pass to
    /// [`wait_durable`](Self::wait_durable) once the gate is released.
    async fn publish(&self, entries: Vec<WalEntryData>) -> Result<Option<u64>, String> {
        let mut ticket = Ok(None);
        if let Some(wal) = self.wal
            && !entries.is_empty()
        {
            ticket = wal.append(&entries).map(Some).map_err(|e| {
                tracing::error!(error = %e, "failed to append to WAL");
                format!("write was applied but could not be written to the WAL: {e}")
            });
        }
        if let Some(repl) = self.replication {
            emit_wal_entries(entries, repl).await;
        }
        ticket
    }

    /// Wait until the WAL is on disk up to `ticket` (group commit).
    async fn wait_durable(&self, ticket: Option<u64>) -> Result<(), String> {
        let (Some(wal), Some(offset)) = (self.wal, ticket) else {
            return Ok(());
        };
        let wal = Arc::clone(wal);
        tokio::task::spawn_blocking(move || wal.sync_to(offset))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()))
            .map_err(|e| {
                tracing::error!(error = %e, "failed to sync WAL");
                format!("write was applied but the WAL could not be synced: {e}")
            })
    }
}

/// Schema/index state shared by every query on the server.
///
/// Read-only queries borrow the managers under read locks (no per-query clone).
/// Write queries are serialized by `write_gate` and update the managers in place
/// under write locks, so concurrent writers can no longer overwrite each other's
/// index updates (lost update) and UNIQUE checks see every committed write.
/// The gate is an async mutex so it can also cover the WAL emission that follows
/// execution, keeping replication order identical to execution order.
struct SharedManagers {
    constraints: RwLock<ConstraintManager>,
    fulltext: RwLock<FulltextManager>,
    /// Shared property (B-tree) index so that `CREATE INDEX` and subsequent
    /// index-accelerated lookups survive between requests.
    property_index: RwLock<PropertyIndex>,
    write_gate: Arc<tokio::sync::Mutex<()>>,
}

impl SharedManagers {
    fn new() -> Self {
        Self {
            constraints: RwLock::new(ConstraintManager::new()),
            fulltext: RwLock::new(FulltextManager::new()),
            property_index: RwLock::new(PropertyIndex::new()),
            write_gate: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Run `stmt` against `graph` with these managers.
    ///
    /// Write statements must be executed while holding `write_gate`. For writes
    /// with `record_wal`, the returned log holds every graph mutation in
    /// execution order. A statement that fails midway is undone and returns an
    /// empty log, so the WAL and followers never see its partial changes.
    #[cfg(test)]
    fn execute(
        &self,
        graph: &ConcurrentGraph,
        stmt: maharit_query::ast::Statement,
        is_write: bool,
        record_wal: bool,
    ) -> (
        Result<maharit_query::ResultSet, maharit_query::ExecuteError>,
        Vec<WalEntryData>,
    ) {
        self.execute_with_params(graph, stmt, HashMap::new(), is_write, record_wal)
    }

    /// [`execute`](Self::execute) with `$name` query parameters.
    fn execute_with_params(
        &self,
        graph: &ConcurrentGraph,
        stmt: maharit_query::ast::Statement,
        params: HashMap<String, Value>,
        is_write: bool,
        record_wal: bool,
    ) -> (
        Result<maharit_query::ResultSet, maharit_query::ExecuteError>,
        Vec<WalEntryData>,
    ) {
        let (result, wal, _) = self.run(graph, stmt, params, is_write, record_wal, false);
        (result, wal)
    }

    /// Run a write statement inside a transaction: like [`execute`] but also
    /// returns the undo records needed to roll the statement back.
    ///
    /// Must be called while holding `write_gate`.
    fn execute_in_tx(
        &self,
        graph: &ConcurrentGraph,
        stmt: maharit_query::ast::Statement,
        params: HashMap<String, Value>,
        record_wal: bool,
    ) -> (
        Result<maharit_query::ResultSet, maharit_query::ExecuteError>,
        Vec<WalEntryData>,
        Vec<UndoRecord>,
    ) {
        self.run(graph, stmt, params, true, record_wal, true)
    }

    fn run(
        &self,
        graph: &ConcurrentGraph,
        stmt: maharit_query::ast::Statement,
        params: HashMap<String, Value>,
        is_write: bool,
        record_wal: bool,
        capture_undo: bool,
    ) -> (
        Result<maharit_query::ResultSet, maharit_query::ExecuteError>,
        Vec<WalEntryData>,
        Vec<UndoRecord>,
    ) {
        // Lock order is always constraints → fulltext → property_index.
        // A poisoned lock only means another query panicked mid-execution; the
        // managers are still structurally valid, so keep serving.
        if is_write {
            use maharit_query::ast::Statement;
            let is_ddl = matches!(
                stmt,
                Statement::CreateConstraint(_)
                    | Statement::DropConstraint(_)
                    | Statement::CreateIndex(_)
                    | Statement::DropIndex(_)
                    | Statement::CreateFulltextIndex(_)
                    | Statement::DropFulltextIndex(_)
            );
            let mut cm = self.constraints.write().unwrap_or_else(|e| e.into_inner());
            let mut fm = self.fulltext.write().unwrap_or_else(|e| e.into_inner());
            let mut pi = self
                .property_index
                .write()
                .unwrap_or_else(|e| e.into_inner());
            // Undo records are always captured so that a statement that fails
            // midway can be undone: a statement either applies completely or
            // not at all (bug/160).
            let mut recorder = RecordingGraph::new(graph, record_wal).with_undo();
            let result =
                Executor::new_with_backend_exclusive(&mut recorder, &mut cm, &mut fm, &mut pi)
                    .execute_with_params(stmt, params);
            let (mut wal, mut undo) = recorder.into_parts();
            if result.is_err() {
                if !undo.is_empty() {
                    // Not recorded: the partial changes never reach the WAL or
                    // followers, so nothing needs to be compensated there.
                    let mut reverter = RecordingGraph::new(graph, false);
                    let touched = maharit_storage::apply_undo(&undo, &mut reverter);
                    crate::schema::reindex_nodes(graph, touched, &mut fm, &mut pi);
                }
                wal.clear();
                undo.clear();
            } else if !capture_undo {
                undo.clear();
            }
            // Log the whole schema after a DDL statement so that WAL replay
            // and followers end up with the same constraints and indexes.
            if is_ddl && record_wal && result.is_ok() {
                wal.push(WalEntryData::SetSchema {
                    schema: Schema::export(&cm, &fm, &pi),
                });
            }
            (result, wal, undo)
        } else {
            let cm = self.constraints.read().unwrap_or_else(|e| e.into_inner());
            let fm = self.fulltext.read().unwrap_or_else(|e| e.into_inner());
            let pi = self
                .property_index
                .read()
                .unwrap_or_else(|e| e.into_inner());
            // SAFETY: ConcurrentGraph has interior mutability via DashMap; the
            // executor uses the raw pointer only during this synchronous call.
            let mut executor = unsafe { Executor::new_concurrent_shared(graph, &cm, &fm, &pi) };
            (
                executor.execute_with_params(stmt, params),
                Vec::new(),
                Vec::new(),
            )
        }
    }

    /// Roll back transaction `tx_id`: apply its undo log to the graph, bring
    /// the property / fulltext indexes of every touched node back in line with
    /// the restored state, and return the compensating mutations as WAL
    /// entries so followers roll back too.
    ///
    /// Must be called while holding `write_gate`.
    fn rollback(
        &self,
        graph: &ConcurrentGraph,
        tx_manager: &TransactionManager,
        tx_id: u64,
        record_wal: bool,
    ) -> Result<Vec<WalEntryData>, maharit_storage::TransactionError> {
        let mut fm = self.fulltext.write().unwrap_or_else(|e| e.into_inner());
        let mut pi = self
            .property_index
            .write()
            .unwrap_or_else(|e| e.into_inner());
        let mut recorder = RecordingGraph::new(graph, record_wal);
        let touched = tx_manager.rollback_backend(tx_id, &mut recorder)?;
        crate::schema::reindex_nodes(graph, touched, &mut fm, &mut pi);
        Ok(recorder.into_log())
    }

    /// The current constraint and index definitions.
    fn export_schema(&self) -> Schema {
        let cm = self.constraints.read().unwrap_or_else(|e| e.into_inner());
        let fm = self.fulltext.read().unwrap_or_else(|e| e.into_inner());
        let pi = self
            .property_index
            .read()
            .unwrap_or_else(|e| e.into_inner());
        Schema::export(&cm, &fm, &pi)
    }

    /// Replace the definitions with `schema` and rebuild the indexes.
    fn install_schema(&self, graph: &ConcurrentGraph, schema: &Schema) -> Result<(), String> {
        let mut cm = self.constraints.write().unwrap_or_else(|e| e.into_inner());
        let mut fm = self.fulltext.write().unwrap_or_else(|e| e.into_inner());
        let mut pi = self
            .property_index
            .write()
            .unwrap_or_else(|e| e.into_inner());
        schema.install(graph, &mut cm, &mut fm, &mut pi)
    }
}

impl ReplicaApply for SharedManagers {
    fn applied(&self, graph: &ConcurrentGraph, entry: &WalEntryData) {
        if let WalEntryData::SetSchema { schema } = entry {
            if let Err(e) = self.install_schema(graph, schema) {
                tracing::error!(error = %e, "replication: failed to apply schema");
            }
            return;
        }
        if let Some(id) = crate::replication::touched_node(entry) {
            let mut fm = self.fulltext.write().unwrap_or_else(|e| e.into_inner());
            let mut pi = self
                .property_index
                .write()
                .unwrap_or_else(|e| e.into_inner());
            if pi.has_any_index() || !fm.list_indexes().is_empty() {
                crate::schema::reindex_nodes(graph, [id], &mut fm, &mut pi);
            }
        }
    }

    fn snapshot_applied(&self, graph: &ConcurrentGraph, schema: &Schema) {
        if let Err(e) = self.install_schema(graph, schema) {
            tracing::error!(error = %e, "replication: failed to apply schema from snapshot");
        }
    }
}

/// Capacity (number of distinct queries) of the shared AST cache.
const AST_CACHE_CAPACITY: usize = 512;

impl TcpServer {
    /// Create a new TCP server
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            graph: Arc::new(ConcurrentGraph::new()),
            stats: Arc::new(ServerStats::default()),
            shutdown: Arc::new(AtomicBool::new(false)),
            tx_manager: Arc::new(TransactionManager::new()),
            managers: Arc::new(SharedManagers::new()),
            replication: SharedLeader::default(),
            follower: None,
            promotion: None,
            auth: Arc::new(Mutex::new(crate::auth::AuthManager::new())),
            ast_cache: Arc::new(Mutex::new(AstCache::new(AST_CACHE_CAPACITY))),
            durability: None,
            tls: None,
            metrics: Arc::new(crate::metrics::Metrics::new()),
            audit: None,
        }
    }

    /// Create a server with an existing graph
    pub fn with_graph(config: ServerConfig, graph: ConcurrentGraph) -> Self {
        Self {
            config,
            graph: Arc::new(graph),
            stats: Arc::new(ServerStats::default()),
            shutdown: Arc::new(AtomicBool::new(false)),
            tx_manager: Arc::new(TransactionManager::new()),
            managers: Arc::new(SharedManagers::new()),
            replication: SharedLeader::default(),
            follower: None,
            promotion: None,
            auth: Arc::new(Mutex::new(crate::auth::AuthManager::new())),
            ast_cache: Arc::new(Mutex::new(AstCache::new(AST_CACHE_CAPACITY))),
            durability: None,
            tls: None,
            metrics: Arc::new(crate::metrics::Metrics::new()),
            audit: None,
        }
    }

    /// Create a server with a shared graph Arc (for sharing with signal handlers)
    pub fn with_graph_arc(config: ServerConfig, graph: Arc<ConcurrentGraph>) -> Self {
        Self {
            config,
            graph,
            stats: Arc::new(ServerStats::default()),
            shutdown: Arc::new(AtomicBool::new(false)),
            tx_manager: Arc::new(TransactionManager::new()),
            managers: Arc::new(SharedManagers::new()),
            replication: SharedLeader::default(),
            follower: None,
            promotion: None,
            auth: Arc::new(Mutex::new(crate::auth::AuthManager::new())),
            ast_cache: Arc::new(Mutex::new(AstCache::new(AST_CACHE_CAPACITY))),
            durability: None,
            tls: None,
            metrics: Arc::new(crate::metrics::Metrics::new()),
            audit: None,
        }
    }

    /// The metrics this server records (queries, errors, latency, connections,
    /// graph size). Hand it to the monitoring HTTP server.
    pub fn metrics(&self) -> Arc<crate::metrics::Metrics> {
        self.metrics.set_node_count(self.graph.node_count() as i64);
        self.metrics.set_edge_count(self.graph.edge_count() as i64);
        Arc::clone(&self.metrics)
    }

    /// Return a clone of the graph Arc
    pub fn graph_arc(&self) -> Arc<ConcurrentGraph> {
        Arc::clone(&self.graph)
    }

    /// Replace the authentication manager (e.g. to set a non-default admin
    /// password before enabling `require_auth`).
    pub fn with_auth(mut self, auth: crate::auth::AuthManager) -> Self {
        self.auth = Arc::new(Mutex::new(auth));
        self
    }

    /// 監査ログを有効にする（ログイン成功・失敗、ユーザー管理、権限エラーを記録）。
    pub fn with_audit_log(mut self, logger: crate::audit::AuditLogger) -> Self {
        self.audit = Some(Arc::new(logger));
        self
    }

    /// Attach a leader replication manager.  Once attached, every successful
    /// write query automatically appends WAL entries to the manager, which
    /// broadcasts them to all connected followers.
    pub fn with_replication(self, manager: Arc<LeaderReplicationManager>) -> Self {
        manager.set_snapshot_source(snapshot_source(&self.graph, &self.managers));
        *self.replication.write().unwrap_or_else(|e| e.into_inner()) = Some(manager);
        self
    }

    /// Let a follower be promoted with the `promote` request: it then stops
    /// following, accepts writes and serves replication as `promotion` says.
    pub fn with_promotion(mut self, promotion: Promotion) -> Self {
        self.promotion = Some(Arc::new(promotion));
        self
    }

    /// Log every write to an on-disk WAL before replying, and checkpoint it
    /// into the snapshot. Do not use on a follower: replicated changes bypass
    /// the WAL.
    pub fn with_durability(mut self, durability: Durability) -> Self {
        self.durability = Some(Arc::new(durability));
        self
    }

    /// A handle that checkpoints this server's snapshot + WAL (e.g. on
    /// shutdown). `None` when no WAL is configured.
    pub fn checkpointer(&self) -> Option<Checkpointer> {
        self.durability.as_ref().map(|d| Checkpointer {
            graph: Arc::clone(&self.graph),
            managers: Arc::clone(&self.managers),
            durability: Arc::clone(d),
        })
    }

    /// Serve client connections over TLS. Connections that do not complete a
    /// TLS handshake within `read_timeout` are closed.
    pub fn with_tls(mut self, tls: ReloadableTlsAcceptor) -> Self {
        self.tls = Some(tls);
        self
    }

    /// Attach a follower replication manager so the `stats` response can report
    /// this follower's view of leader liveness.
    pub fn with_follower(mut self, manager: Arc<FollowerReplicationManager>) -> Self {
        manager.set_apply_hook(Arc::clone(&self.managers) as Arc<dyn ReplicaApply>);
        self.follower = Some(manager);
        self
    }

    /// Install `schema` (constraints and indexes, e.g. loaded from the data
    /// file and WAL) and build its indexes from the current graph.
    pub fn with_schema(self, schema: &Schema) -> Result<Self, String> {
        self.managers.install_schema(&self.graph, schema)?;
        Ok(self)
    }

    /// Saves the graph and schema to a data file (for shutdown without a WAL).
    pub fn snapshot_writer(&self) -> SnapshotWriter {
        SnapshotWriter {
            graph: Arc::clone(&self.graph),
            managers: Arc::clone(&self.managers),
        }
    }

    /// Start the server
    pub async fn start(&self) -> std::io::Result<()> {
        // Initialise structured tracing (JSON to stderr; honours RUST_LOG env var)
        let _tracing_guard = TracingConfig::default().init();

        let listener = TcpListener::bind(&self.config.bind_address).await?;
        let transport = if self.tls.is_some() {
            "TLS"
        } else {
            "plaintext"
        };
        tracing::info!(address = %self.config.bind_address, transport, "Server listening");
        println!(
            "Server listening on {} ({})",
            self.config.bind_address, transport
        );
        if self.tls.is_none() && !is_loopback_bind(&self.config.bind_address) {
            tracing::warn!(
                "client connections are NOT encrypted; use --tls-cert/--tls-key when the port is reachable from other hosts"
            );
            eprintln!(
                "WARN: client connections are NOT encrypted (no --tls-cert/--tls-key) and the server listens on {}.",
                self.config.bind_address
            );
        }

        // 認証無効時に警告を出す（運用者が見落とさないよう WARN レベル）
        if !self.config.require_auth {
            tracing::warn!(
                "maharit-server authentication is DISABLED. All requests are accepted without sessionToken. Set ServerConfig::require_auth=true to enforce login."
            );
            eprintln!(
                "WARN: maharit-server authentication is DISABLED. All requests are accepted without sessionToken."
            );
        }

        self.start_with_listener(listener).await
    }

    /// Start the server with an existing listener.
    ///
    /// Useful for testing: bind to port 0 externally, retrieve the actual
    /// address via `listener.local_addr()`, then pass the listener here.
    pub async fn start_with_listener(&self, listener: TcpListener) -> std::io::Result<()> {
        // Create shutdown broadcast channel
        let (shutdown_tx, _) = broadcast::channel::<()>(1);

        if let Some(tls) = &self.tls {
            tls.spawn_watcher(DEFAULT_TLS_RELOAD_INTERVAL, Arc::clone(&self.shutdown));
        }

        // Checkpoint whenever the WAL grows past the configured size.
        if let Some(checkpointer) = self.checkpointer()
            && checkpointer.durability.checkpoint_bytes > 0
        {
            let shutdown = Arc::clone(&self.shutdown);
            tokio::spawn(async move {
                let limit = checkpointer.durability.checkpoint_bytes;
                let mut ticker = tokio::time::interval(Duration::from_secs(1));
                while !shutdown.load(Ordering::SeqCst) {
                    ticker.tick().await;
                    if checkpointer.durability.wal.len() < limit {
                        continue;
                    }
                    match checkpointer.checkpoint().await {
                        Ok(()) => tracing::info!("checkpoint completed"),
                        Err(e) => tracing::error!(error = %e, "checkpoint failed"),
                    }
                }
            });
        }

        loop {
            if self.shutdown.load(Ordering::SeqCst) {
                println!("Server shutting down...");
                break;
            }

            // Check connection limit
            let current = self.stats.current_connections.load(Ordering::SeqCst);
            if current >= self.config.max_connections as u64 {
                // Wait a bit before accepting more connections
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }

            // Accept with timeout to allow checking shutdown flag
            let accept_result = timeout(Duration::from_secs(1), listener.accept()).await;

            match accept_result {
                Ok(Ok((socket, addr))) => {
                    tracing::info!(peer = %addr, "Client connected");
                    self.stats.total_connections.fetch_add(1, Ordering::SeqCst);
                    self.stats
                        .current_connections
                        .fetch_add(1, Ordering::SeqCst);
                    self.metrics.increment_connections();
                    let metrics = Arc::clone(&self.metrics);

                    let graph = Arc::clone(&self.graph);
                    let stats = Arc::clone(&self.stats);
                    let shutdown = Arc::clone(&self.shutdown);
                    let tx_manager = Arc::clone(&self.tx_manager);
                    let managers = Arc::clone(&self.managers);
                    let config = self.config.clone();
                    let replication = Arc::clone(&self.replication);
                    let follower = self.follower.clone();
                    let promotion = self.promotion.clone();
                    let auth = Arc::clone(&self.auth);
                    let ast_cache = Arc::clone(&self.ast_cache);
                    let durability = self.durability.clone();
                    let audit = self.audit.clone();
                    let tls = self.tls.as_ref().map(ReloadableTlsAcceptor::acceptor);
                    let mut shutdown_rx = shutdown_tx.subscribe();

                    tokio::spawn(async move {
                        let ctx = ConnectionContext {
                            graph,
                            stats: stats.clone(),
                            shutdown,
                            tx_manager,
                            managers,
                            config,
                            replication,
                            follower,
                            promotion,
                            auth,
                            ast_cache,
                            durability,
                            metrics: Arc::clone(&metrics),
                            audit,
                        };
                        let result = match tls {
                            None => handle_connection(socket, ctx, &mut shutdown_rx).await,
                            // The handshake runs in the connection task so a slow
                            // client cannot stall the accept loop.
                            Some(acceptor) => {
                                match timeout(ctx.config.read_timeout, acceptor.accept(socket))
                                    .await
                                {
                                    Ok(Ok(tls_stream)) => {
                                        handle_connection(tls_stream, ctx, &mut shutdown_rx).await
                                    }
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
                            tracing::warn!(peer = %addr, error = %e, "Connection error");
                            eprintln!("Connection error from {}: {}", addr, e);
                        } else {
                            tracing::info!(peer = %addr, "Client disconnected");
                        }

                        stats.current_connections.fetch_sub(1, Ordering::SeqCst);
                        metrics.decrement_connections();
                    });
                }
                Ok(Err(e)) => {
                    eprintln!("Accept error: {}", e);
                }
                Err(_) => {
                    // Timeout, just continue to check shutdown flag
                }
            }
        }

        Ok(())
    }

    /// Request graceful shutdown
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Get current statistics
    pub fn stats(&self) -> &ServerStats {
        &self.stats
    }
}

/// Whether `bind_address` only accepts connections from this host.
pub(crate) fn is_loopback_bind(bind_address: &str) -> bool {
    let host = bind_address
        .rsplit_once(':')
        .map_or(bind_address, |(h, _)| h)
        .trim_start_matches('[')
        .trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Server state shared with one connection task.
struct ConnectionContext {
    graph: Arc<ConcurrentGraph>,
    stats: Arc<ServerStats>,
    shutdown: Arc<AtomicBool>,
    tx_manager: Arc<TransactionManager>,
    managers: Arc<SharedManagers>,
    config: ServerConfig,
    replication: SharedLeader,
    follower: Option<Arc<FollowerReplicationManager>>,
    promotion: Option<Arc<Promotion>>,
    auth: Arc<Mutex<crate::auth::AuthManager>>,
    ast_cache: Arc<Mutex<AstCache>>,
    durability: Option<Arc<Durability>>,
    metrics: Arc<crate::metrics::Metrics>,
    audit: Option<Arc<crate::audit::AuditLogger>>,
}

/// Handle a single client connection (plain TCP or TLS).
async fn handle_connection<S>(
    mut socket: S,
    ctx: ConnectionContext,
    shutdown_rx: &mut broadcast::Receiver<()>,
) -> std::io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let ConnectionContext {
        graph,
        stats,
        shutdown,
        tx_manager,
        managers,
        config,
        replication,
        follower,
        promotion,
        auth,
        ast_cache,
        durability,
        metrics,
        audit,
    } = ctx;
    let audit = audit.as_deref();
    let mut buffer = BytesMut::with_capacity(4096);

    // Transactions begun on this connection and not finished yet: rolled back
    // when the connection ends (bug/159).
    let mut open_txs: Vec<u64> = Vec::new();
    let result: std::io::Result<()> = async {
        loop {
            // Read per request: a promoted follower starts publishing to its own
            // followers from the next request on.
            let leader = replication
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let sinks = WriteSinks {
                replication: leader.as_deref(),
                wal: durability.as_ref().map(|d| &d.wal),
            };

            if shutdown.load(Ordering::SeqCst) {
                send_response(&mut socket, &Response::Goodbye, config.write_timeout).await?;
                break;
            }

            // Read with timeout
            let read_result = tokio::select! {
                result = read_message(&mut socket, &mut buffer, config.read_timeout) => result,
                _ = shutdown_rx.recv() => {
                    send_response(&mut socket, &Response::Goodbye, config.write_timeout).await?;
                    break;
                }
            };

            let message = match read_result {
                Ok(Some(msg)) => msg,
                Ok(None) => break, // Connection closed
                Err(e) => {
                    let response = Response::Error {
                        message: e.to_string(),
                    };
                    send_response(&mut socket, &response, config.write_timeout).await?;
                    continue;
                }
            };

            // Parse request
            let request: Request = match serde_json::from_slice(&message) {
                Ok(req) => req,
                Err(e) => {
                    let response = Response::Error {
                        message: format!("Invalid request: {}", e),
                    };
                    send_response(&mut socket, &response, config.write_timeout).await?;
                    continue;
                }
            };

            // Handle request
            // Metrics: classify the query before `request` is consumed.
            let measured = match &request {
                Request::Query { query, .. } | Request::StreamQuery { query, .. } => {
                    Some(crate::metrics::QueryType::of_query(query))
                }
                _ => None,
            };
            let started = std::time::Instant::now();

            let response = match request {
                Request::Login { username, password } => {
                    let mut mgr = auth.lock().unwrap();
                    match mgr.authenticate(&username, &password) {
                        Ok(token) => {
                            audit_log(
                                audit,
                                crate::audit::AuthEvent::LoginSuccess {
                                    username: username.clone(),
                                },
                            );
                            let role = mgr
                                .validate_session(&token)
                                .map(|s| role_label(&s.role))
                                .unwrap_or_else(|_| "unknown".to_string());
                            let expires_at = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0)
                                + DEFAULT_SESSION_TIMEOUT_SECS;
                            Response::LoggedIn {
                                session_token: token,
                                role,
                                expires_at,
                            }
                        }
                        Err(e) => {
                            audit_log(
                                audit,
                                crate::audit::AuthEvent::LoginFailure {
                                    username: username.clone(),
                                },
                            );
                            Response::AuthError {
                                message: format!("{}", e),
                            }
                        }
                    }
                }
                Request::Query {
                    query,
                    params,
                    tx_id,
                    session_token,
                } => match check_session_user(config.require_auth, &auth, &session_token) {
                    Err(resp) => resp,
                    Ok((role, user)) => {
                        if let Some(resp) = try_handle_user_management(
                            &query,
                            &auth,
                            &ast_cache,
                            role,
                            config.require_auth,
                            audit,
                            &user,
                        ) {
                            resp
                        } else if let Some(resp) = authorize_query(role, &query) {
                            audit_denied(audit, &user, "write");
                            resp
                        } else if let Some(resp) =
                            reject_write_on_follower(follower.as_deref(), &ast_cache, &query)
                        {
                            resp
                        } else {
                            stats.total_queries.fetch_add(1, Ordering::SeqCst);
                            // `.instrument` (not `span.enter()`): the guard of
                            // `enter` must not be held across `.await`, or the
                            // span can be closed on another worker thread.
                            let span = tracing::info_span!("query", query = %query);
                            let params = params_from_json(&params);
                            async {
                                let start = std::time::Instant::now();
                                let resp = match tx_id {
                                    Some(id) => {
                                        execute_query_with_tx(
                                            &graph,
                                            &query,
                                            params,
                                            id,
                                            &tx_manager,
                                            &managers,
                                            sinks,
                                            &ast_cache,
                                        )
                                        .await
                                    }
                                    None => {
                                        execute_query(
                                            &graph, &query, params, &managers, sinks, &ast_cache,
                                        )
                                        .await
                                    }
                                };
                                tracing::info!(
                                    duration_us = start.elapsed().as_micros() as u64,
                                    "query completed"
                                );
                                resp
                            }
                            .instrument(span)
                            .await
                        }
                    }
                },
                Request::StreamQuery {
                    query,
                    params,
                    tx_id: _,
                    chunk_size,
                    session_token,
                } => match check_session_user(config.require_auth, &auth, &session_token) {
                    Err(resp) => resp,
                    Ok((role, user)) if authorize_query(role, &query).is_some() => {
                        audit_denied(audit, &user, "write");
                        authorize_query(role, &query).unwrap()
                    }
                    Ok(_)
                        if reject_write_on_follower(follower.as_deref(), &ast_cache, &query)
                            .is_some() =>
                    {
                        reject_write_on_follower(follower.as_deref(), &ast_cache, &query).unwrap()
                    }
                    Ok(_) => {
                        stats.total_queries.fetch_add(1, Ordering::SeqCst);
                        tracing::info!(query = %query, "streaming query");
                        // Execute streaming query
                        if let Err(e) = execute_streaming_query(
                            &mut socket,
                            &graph,
                            &stats,
                            &query,
                            params_from_json(&params),
                            chunk_size,
                            config.write_timeout,
                            &managers,
                            sinks,
                            &ast_cache,
                        )
                        .await
                        {
                            Response::Error {
                                message: format!("Streaming error: {}", e),
                            }
                        } else {
                            // Streaming responses already sent, continue to next request
                            continue;
                        }
                    }
                },
                Request::Ping => Response::Pong,
                Request::Stats => {
                    // Report replication status (role, node id, current LSN, follower
                    // count, leader liveness) when this node participates in
                    // replication as a follower or leader.
                    let replication_status = match (leader.as_ref(), follower.as_ref()) {
                        (Some(l), _) => Some(ReplicationStatus::from(l.get_stats())),
                        (None, Some(f)) if f.is_promoted() => Some(ReplicationStatus {
                            role: "leader".to_string(),
                            node_id: f.get_stats().node_id,
                            current_lsn: 0,
                            follower_count: 0,
                            is_leader_alive: true,
                        }),
                        (None, Some(f)) => Some(ReplicationStatus::from(f.get_stats())),
                        (None, None) => None,
                    };
                    Response::Stats {
                        connections: stats.current_connections.load(Ordering::SeqCst),
                        total_queries: stats.total_queries.load(Ordering::SeqCst),
                        nodes: graph.node_count(),
                        edges: graph.edge_count(),
                        replication: replication_status,
                    }
                }
                Request::Promote { session_token } => {
                    match check_session_user(config.require_auth, &auth, &session_token) {
                        Err(resp) => resp,
                        Ok((role, user)) if role != crate::auth::Role::Admin => {
                            audit_denied(audit, &user, "promote");
                            Response::AuthError {
                                message: "promote requires the admin role".to_string(),
                            }
                        }
                        Ok(_) => {
                            promote_to_leader(
                                follower.as_deref(),
                                promotion.as_deref(),
                                &replication,
                                &graph,
                                &managers,
                                durability.as_ref(),
                            )
                            .await
                        }
                    }
                }
                Request::Disconnect => {
                    send_response(&mut socket, &Response::Goodbye, config.write_timeout).await?;
                    break;
                }
                Request::BeginTransaction {
                    read_only,
                    session_token,
                } => match check_session_user(config.require_auth, &auth, &session_token) {
                    Err(resp) => resp,
                    Ok((role, user)) => {
                        // 書き込みトランザクションの開始には Write 権限を要求する。
                        if !read_only
                            && crate::auth::AuthManager::check_role_permission(
                                role,
                                crate::auth::Operation::Write,
                            )
                            .is_err()
                        {
                            audit_denied(audit, &user, "begin_write_transaction");
                            Response::AuthError {
                                message: "permission denied: role cannot begin a write transaction"
                                    .to_string(),
                            }
                        } else {
                            let tx_id = if read_only {
                                tx_manager.begin_read_only()
                            } else {
                                tx_manager.begin()
                            };
                            Response::TransactionBegun { tx_id }
                        }
                    }
                },
                Request::Commit {
                    tx_id,
                    session_token,
                } => match check_session(config.require_auth, &auth, &session_token) {
                    Err(resp) => resp,
                    Ok(_) => match tx_manager.commit(tx_id) {
                        Ok(()) => Response::Committed { tx_id },
                        Err(e) => Response::Error {
                            message: format!("Commit failed: {}", e),
                        },
                    },
                },
                Request::Rollback {
                    tx_id,
                    session_token,
                } => match check_session(config.require_auth, &auth, &session_token) {
                    Err(resp) => resp,
                    Ok(_) => {
                        rollback_transaction(&graph, &managers, &tx_manager, sinks, tx_id).await
                    }
                },
            };

            if let Some(query_type) = measured {
                metrics.record_query(query_type, started.elapsed());
                if matches!(
                    response,
                    Response::Error { .. } | Response::AuthError { .. }
                ) {
                    metrics.record_error();
                }
                metrics.set_node_count(graph.node_count() as i64);
                metrics.set_edge_count(graph.edge_count() as i64);
            }

            match &response {
                Response::TransactionBegun { tx_id } => open_txs.push(*tx_id),
                Response::Committed { tx_id } | Response::RolledBack { tx_id } => {
                    open_txs.retain(|t| t != tx_id)
                }
                _ => {}
            }

            send_response(&mut socket, &response, config.write_timeout).await?;
        }
        Ok(())
    }
    .await;

    if !open_txs.is_empty() {
        let leader = replication
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let sinks = WriteSinks {
            replication: leader.as_deref(),
            wal: durability.as_ref().map(|d| &d.wal),
        };
        for tx_id in open_txs {
            if !tx_manager.is_active(tx_id) {
                continue;
            }
            if let Response::Error { message } =
                rollback_transaction(&graph, &managers, &tx_manager, sinks, tx_id).await
            {
                tracing::warn!(tx_id, error = %message, "failed to roll back on disconnect");
            } else {
                tracing::info!(tx_id, "rolled back unfinished transaction on disconnect");
            }
        }
    }

    result
}

/// Roll back `tx_id`: apply the compensating changes, write them to the WAL and
/// send them to followers.
async fn rollback_transaction(
    graph: &Arc<ConcurrentGraph>,
    managers: &SharedManagers,
    tx_manager: &TransactionManager,
    sinks: WriteSinks<'_>,
    tx_id: u64,
) -> Response {
    // Serialize with writers so the compensating WAL entries are ordered after
    // the statements they undo.
    let write_guard = managers.write_gate.lock().await;
    match managers.rollback(graph, tx_manager, tx_id, sinks.record()) {
        Ok(wal) => {
            let ticket = sinks.publish(wal).await;
            drop(write_guard);
            let durable = match ticket {
                Ok(t) => sinks.wait_durable(t).await,
                Err(e) => Err(e),
            };
            match durable {
                Ok(()) => Response::RolledBack { tx_id },
                Err(message) => Response::Error { message },
            }
        }
        Err(e) => Response::Error {
            message: format!("Rollback failed: {}", e),
        },
    }
}

/// Read a length-prefixed message from the socket
async fn read_message<S: AsyncRead + Unpin>(
    socket: &mut S,
    buffer: &mut BytesMut,
    read_timeout: Duration,
) -> std::io::Result<Option<Vec<u8>>> {
    loop {
        // Check if we have a complete message in the buffer
        if buffer.len() >= 4 {
            let len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;

            // 宣言された長さが上限を超える場合は、データを蓄積する前に切断する。
            // これがないと最大約 4 GiB のバッファ確保を強制されメモリ枯渇 DoS になる。
            if len > MAX_MESSAGE_SIZE {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "message length {} exceeds maximum {}",
                        len, MAX_MESSAGE_SIZE
                    ),
                ));
            }

            if buffer.len() >= 4 + len {
                buffer.advance(4);
                let message = buffer.split_to(len).to_vec();
                return Ok(Some(message));
            }
        }

        // Read more data
        let n = match timeout(read_timeout, socket.read_buf(buffer)).await {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Read timeout",
                ));
            }
        };

        if n == 0 {
            return Ok(None); // Connection closed
        }
    }
}

/// Send a response with length prefix
async fn send_response<S: AsyncWrite + Unpin>(
    socket: &mut S,
    response: &Response,
    write_timeout: Duration,
) -> std::io::Result<()> {
    let json = serde_json::to_vec(response)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

    let len = json.len() as u32;
    let len_bytes = len.to_be_bytes();

    let write_future = async {
        socket.write_all(&len_bytes).await?;
        socket.write_all(&json).await?;
        socket.flush().await
    };

    match timeout(write_timeout, write_future).await {
        Ok(result) => result,
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "Write timeout",
        )),
    }
}

/// Execute a streaming query and send results in chunks
#[allow(clippy::too_many_arguments)]
async fn execute_streaming_query<S: AsyncWrite + Unpin>(
    socket: &mut S,
    graph: &Arc<ConcurrentGraph>,
    stats: &Arc<ServerStats>,
    query: &str,
    params: HashMap<String, Value>,
    chunk_size: usize,
    write_timeout: Duration,
    managers: &SharedManagers,
    sinks: WriteSinks<'_>,
    ast_cache: &Arc<Mutex<AstCache>>,
) -> std::io::Result<()> {
    // Parse the query (reusing a cached AST when available). Bind the result to
    // a local so the mutex guard is released before any `.await` below.
    let parsed = ast_cache.lock().unwrap().get_or_parse(query);
    let stmt = match parsed {
        Ok(stmt) => stmt,
        Err(e) => {
            let response = Response::Error {
                message: format!("Parse error: {}", e),
            };
            return send_response(socket, &response, write_timeout).await;
        }
    };

    let is_write = !is_read_only(&stmt);

    // Serialize writers across execute → WAL emission (see SharedManagers).
    let write_guard = if is_write {
        Some(managers.write_gate.lock().await)
    } else {
        None
    };

    let (exec_result, wal) =
        managers.execute_with_params(graph, stmt, params, is_write, sinks.record());
    let ticket = sinks.publish(wal).await;
    drop(write_guard);
    let durable = match ticket {
        Ok(t) => sinks.wait_durable(t).await,
        Err(e) => Err(e),
    };
    if let Err(message) = durable {
        return send_response(socket, &Response::Error { message }, write_timeout).await;
    }

    let result = match exec_result {
        Ok(r) => r,
        Err(e) => {
            let response = Response::Error {
                message: format!("Execution error: {}", e),
            };
            return send_response(socket, &response, write_timeout).await;
        }
    };

    // Convert rows to HashMap<String, serde_json::Value> 型情報維持
    let all_rows: Vec<HashMap<String, serde_json::Value>> = result
        .rows
        .into_iter()
        .map(|row| {
            result
                .columns
                .iter()
                .zip(row.columns.iter())
                .map(|(col, val)| (col.clone(), val.to_json()))
                .collect()
        })
        .collect();

    let total_rows = all_rows.len();
    let stream_id = stats.next_stream_id();

    // Send StreamStart
    let start_response = Response::StreamStart {
        stream_id,
        total_rows: Some(total_rows),
    };
    send_response(socket, &start_response, write_timeout).await?;

    // Send chunks
    let chunk_size = if chunk_size == 0 {
        DEFAULT_CHUNK_SIZE
    } else {
        chunk_size
    };

    for (chunk_index, chunk) in all_rows.chunks(chunk_size).enumerate() {
        let chunk_response = Response::StreamChunk {
            stream_id,
            chunk_index,
            rows: chunk.to_vec(),
        };
        send_response(socket, &chunk_response, write_timeout).await?;
    }

    // Send StreamEnd
    let end_response = Response::StreamEnd {
        stream_id,
        total_rows,
    };
    send_response(socket, &end_response, write_timeout).await?;

    Ok(())
}

/// Convert the JSON `params` of a request to executor values.
fn params_from_json(params: &HashMap<String, serde_json::Value>) -> HashMap<String, Value> {
    params
        .iter()
        .map(|(k, v)| (k.clone(), Value::from_json(v)))
        .collect()
}

// ── Transaction-aware query execution ────────────────────────────────────────

/// Execute a write query within a transaction: execute → record undo log → replicate.
#[allow(clippy::too_many_arguments)]
async fn execute_query_with_tx(
    graph: &Arc<ConcurrentGraph>,
    query: &str,
    params: HashMap<String, Value>,
    tx_id: u64,
    tx_manager: &TransactionManager,
    managers: &SharedManagers,
    sinks: WriteSinks<'_>,
    ast_cache: &Arc<Mutex<AstCache>>,
) -> Response {
    let stmt = match ast_cache.lock().unwrap().get_or_parse(query) {
        Ok(s) => s,
        Err(e) => {
            return Response::Error {
                message: format!("Parse error: {}", e),
            };
        }
    };

    let is_write = !is_read_only(&stmt);

    if !is_write {
        return execute_query(graph, query, params, managers, sinks, ast_cache).await;
    }

    // Serialize writers across execute → WAL emission (see SharedManagers).
    let write_guard = managers.write_gate.lock().await;

    // Refuse to write under a transaction that is unknown or already
    // finished: its changes could never be rolled back.
    if !tx_manager.is_active(tx_id) {
        return Response::Error {
            message: format!("Execution error: transaction {} is not active", tx_id),
        };
    }
    if tx_manager.is_read_only(tx_id) {
        return Response::Error {
            message: format!(
                "Execution error: transaction {} is read-only: cannot run a write query",
                tx_id
            ),
        };
    }

    let (exec_result, wal, undo) = managers.execute_in_tx(graph, stmt, params, sinks.record());
    // A failed statement has already been undone (no undo records left).
    if let Err(e) = tx_manager.record_undo(tx_id, undo) {
        tracing::error!(tx_id, error = %e, "failed to record undo log");
    }
    let ticket = sinks.publish(wal).await;
    drop(write_guard);
    let durable = match ticket {
        Ok(t) => sinks.wait_durable(t).await,
        Err(e) => Err(e),
    };
    if let Err(message) = durable {
        return Response::Error { message };
    }

    match exec_result {
        Ok(result) => {
            let rows: Vec<HashMap<String, serde_json::Value>> = result
                .rows
                .into_iter()
                .map(|row| {
                    result
                        .columns
                        .iter()
                        .zip(row.columns.iter())
                        .map(|(col, val)| (col.clone(), val.to_json()))
                        .collect()
                })
                .collect();
            Response::Result { rows }
        }
        Err(e) => Response::Error {
            message: format!("Execution error: {}", e),
        },
    }
}

/// Execute a query and return the response.
///
/// ConcurrentGraph has interior mutability via DashMap, so both read and write
/// queries use `Executor::new_concurrent` without any async locking.
async fn execute_query(
    graph: &Arc<ConcurrentGraph>,
    query: &str,
    params: HashMap<String, Value>,
    managers: &SharedManagers,
    sinks: WriteSinks<'_>,
    ast_cache: &Arc<Mutex<AstCache>>,
) -> Response {
    // Reuse a previously parsed AST for identical query text.
    let stmt = match ast_cache.lock().unwrap().get_or_parse(query) {
        Ok(stmt) => stmt,
        Err(e) => {
            return Response::Error {
                message: format!("Parse error: {}", e),
            };
        }
    };

    let is_write = !is_read_only(&stmt);

    // Serialize writers across execute → WAL emission (see SharedManagers).
    let write_guard = if is_write {
        Some(managers.write_gate.lock().await)
    } else {
        None
    };

    let (exec_result, wal) =
        managers.execute_with_params(graph, stmt, params, is_write, sinks.record());
    let ticket = sinks.publish(wal).await;
    drop(write_guard);
    let durable = match ticket {
        Ok(t) => sinks.wait_durable(t).await,
        Err(e) => Err(e),
    };
    if let Err(message) = durable {
        return Response::Error { message };
    }

    match exec_result {
        Ok(result) => {
            let rows: Vec<HashMap<String, serde_json::Value>> = result
                .rows
                .into_iter()
                .map(|row| {
                    result
                        .columns
                        .iter()
                        .zip(row.columns.iter())
                        .map(|(col, val)| (col.clone(), val.to_json()))
                        .collect()
                })
                .collect();

            Response::Result { rows }
        }
        Err(e) => Response::Error {
            message: format!("Execution error: {}", e),
        },
    }
}

/// Snapshot source for a leader: this server's graph, write gate and schema.
fn snapshot_source(graph: &Arc<ConcurrentGraph>, managers: &Arc<SharedManagers>) -> SnapshotSource {
    let schema_managers = Arc::clone(managers);
    SnapshotSource {
        graph: Arc::clone(graph),
        write_gate: Arc::clone(&managers.write_gate),
        schema: Arc::new(move || schema_managers.export_schema()),
    }
}

/// Error for a write sent to a follower that has not been promoted, or `None`
/// when the query may run here.
fn reject_write_on_follower(
    follower: Option<&FollowerReplicationManager>,
    ast_cache: &Arc<Mutex<AstCache>>,
    query: &str,
) -> Option<Response> {
    let follower = follower?;
    if follower.is_promoted() {
        return None;
    }
    let stmt = ast_cache.lock().unwrap().get_or_parse(query).ok()?;
    if is_read_only(&stmt) {
        return None;
    }
    Some(Response::Error {
        message: "this node is a read-only follower: send writes to the leader".to_string(),
    })
}

/// Turn this follower into a leader: stop following, make the data file and
/// WAL current, start serving replication (if configured) and accept writes.
async fn promote_to_leader(
    follower: Option<&FollowerReplicationManager>,
    promotion: Option<&Promotion>,
    replication: &SharedLeader,
    graph: &Arc<ConcurrentGraph>,
    managers: &Arc<SharedManagers>,
    durability: Option<&Arc<Durability>>,
) -> Response {
    let Some(follower) = follower else {
        return Response::Error {
            message: "this node is not a follower".to_string(),
        };
    };
    if follower.is_promoted() {
        return Response::Error {
            message: "this node has already been promoted".to_string(),
        };
    }
    let Some(promotion) = promotion else {
        return Response::Error {
            message: "promotion is not configured on this node".to_string(),
        };
    };
    // Bind first: if the address is taken, nothing has changed yet.
    let listener = match &promotion.replication {
        Some(cfg) => match TcpListener::bind(&cfg.replication_bind_address).await {
            Ok(l) => Some(l),
            Err(e) => {
                return Response::Error {
                    message: format!(
                        "cannot listen for followers on {}: {e}",
                        cfg.replication_bind_address
                    ),
                };
            }
        },
        None => None,
    };

    let _gate = managers.write_gate.lock().await;
    follower.stop().await;

    // Everything replicated so far is only in memory: save it as the snapshot
    // and start an empty WAL for it before the first write.
    if let Some(d) = durability {
        let checkpointer = Checkpointer {
            graph: Arc::clone(graph),
            managers: Arc::clone(managers),
            durability: Arc::clone(d),
        };
        if let Err(e) = checkpointer.checkpoint_gated().await {
            tracing::error!(error = %e, "checkpoint during promotion failed");
            return Response::Error {
                message: format!(
                    "promotion stopped replication but failed to save the data file: {e}"
                ),
            };
        }
    }

    let mut replication_bind = None;
    if let (Some(cfg), Some(listener)) = (&promotion.replication, listener) {
        let mut leader = LeaderReplicationManager::new(cfg.clone());
        if let Some(tls) = &promotion.tls {
            leader = leader.with_tls(tls.clone());
        }
        leader.set_snapshot_source(snapshot_source(graph, managers));
        let leader = Arc::new(leader);
        replication_bind = Some(cfg.replication_bind_address.clone());
        if let Err(e) = leader.start_with_listener(listener).await {
            tracing::error!(error = %e, "failed to start the replication listener");
        }
        *replication.write().unwrap_or_else(|e| e.into_inner()) = Some(leader);
    }
    follower.mark_promoted();
    tracing::warn!(replication_bind = ?replication_bind, "promoted to leader");
    println!(
        "Promoted to leader{}",
        replication_bind
            .as_deref()
            .map(|b| format!(" (replication on {b})"))
            .unwrap_or_default()
    );
    Response::Promoted { replication_bind }
}

/// Forward the mutations recorded during a write statement to the WAL, in
/// execution order.
async fn emit_wal_entries(entries: Vec<WalEntryData>, replication: &LeaderReplicationManager) {
    for entry in entries {
        replication.append_wal_entry(entry).await;
    }
}

#[cfg(test)]
#[allow(clippy::approx_constant)] // 3.14 等は PI の近似ではなくリテラルのテストデータ
mod tests {
    use super::*;
    use tokio::net::TcpStream;

    use crate::auth::Role;

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

        let (r, wal) =
            managers.execute(&graph, parse("MATCH (n:P) SET n.age = 2, n:Q"), true, true);
        r.unwrap();
        assert!(wal.iter().any(|e| matches!(
            e,
            WalEntryData::SetProperty { key, value, .. } if key == "age" && value == "2"
        )));
        assert!(wal.iter().any(|e| matches!(
            e,
            WalEntryData::AddLabel { label, .. } if label == "Q"
        )));

        let (r, wal) =
            managers.execute(&graph, parse("MATCH (n:P) REMOVE n.name, n:P"), true, true);
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

    /// Send a request and receive a response over a raw TCP stream.
    async fn send_recv<S: AsyncRead + AsyncWrite + Unpin>(
        stream: &mut S,
        req: &Request,
    ) -> Response {
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
                        tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr))
                            .await;

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

    /// Start a TLS server whose certificate and key are read from the given
    /// files.
    async fn start_tls_server(
        cert: &str,
        key: &str,
    ) -> (std::net::SocketAddr, ReloadableTlsAcceptor) {
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

    #[tokio::test]
    async fn tls_server_serves_queries_and_rejects_plaintext() {
        use crate::tls::test_support::*;
        let cert = TempPem::new("srv_cert", TEST_SERVER_CERT_PEM);
        let key = TempPem::new("srv_key", TEST_SERVER_KEY_PEM);
        let (addr, _) = start_tls_server(cert.path(), key.path()).await;

        let mut stream = tls_connect(addr, cert.path()).await.unwrap();
        assert!(matches!(
            send_recv(&mut stream, &Request::Ping).await,
            Response::Pong
        ));
        let create = Request::Query {
            query: "CREATE (:T {v: 1})".to_string(),
            params: HashMap::new(),
            tx_id: None,
            session_token: None,
        };
        assert!(matches!(
            send_recv(&mut stream, &create).await,
            Response::Result { .. }
        ));

        // A plaintext client gets no protocol response: the server treats its
        // length prefix as a broken TLS record and closes the connection.
        let mut plain = TcpStream::connect(addr).await.unwrap();
        let json = serde_json::to_vec(&Request::Ping).unwrap();
        plain
            .write_all(&(json.len() as u32).to_be_bytes())
            .await
            .unwrap();
        plain.write_all(&json).await.unwrap();
        let mut buf = Vec::new();
        let read = timeout(Duration::from_secs(5), plain.read_to_end(&mut buf)).await;
        let got_pong = matches!(read, Ok(Ok(_)))
            && buf.len() > 4
            && serde_json::from_slice::<Response>(&buf[4..])
                .is_ok_and(|r| matches!(r, Response::Pong));
        assert!(!got_pong, "plaintext request was answered over a TLS port");

        // A client that does not trust the certificate cannot connect.
        let other_ca = TempPem::new("other_ca", TEST_SERVER_CERT2_PEM);
        assert!(tls_connect(addr, other_ca.path()).await.is_err());
    }

    #[tokio::test]
    async fn tls_server_works_with_rust_client() {
        use crate::tls::test_support::*;
        use maharit_client::{Client, TlsClientConfig};
        let cert = TempPem::new("cli_cert", TEST_SERVER_CERT_PEM);
        let key = TempPem::new("cli_key", TEST_SERVER_KEY_PEM);
        let (addr, _) = start_tls_server(cert.path(), key.path()).await;

        let tls = TlsClientConfig::new("localhost").with_ca_cert(cert.path());
        let mut client = Client::connect_tls(&addr.to_string(), tls).await.unwrap();
        client.execute("CREATE (:P {name: 'tls'})").await.unwrap();
        let result = client.query("MATCH (p:P) RETURN p.name").await.unwrap();
        assert_eq!(result.row_count(), 1);
    }

    #[tokio::test]
    async fn tls_certificate_reload_applies_to_new_connections() {
        use crate::tls::test_support::*;
        let cert = TempPem::new("reload_cert", TEST_SERVER_CERT_PEM);
        let key = TempPem::new("reload_key", TEST_SERVER_KEY_PEM);
        let ca1 = TempPem::new("reload_ca1", TEST_SERVER_CERT_PEM);
        let ca2 = TempPem::new("reload_ca2", TEST_SERVER_CERT2_PEM);
        let (addr, tls) = start_tls_server(cert.path(), key.path()).await;

        let mut old_conn = tls_connect(addr, ca1.path()).await.unwrap();
        assert!(tls_connect(addr, ca2.path()).await.is_err());

        // A broken certificate file keeps the previous certificate in use.
        std::fs::write(cert.path(), "not a certificate").unwrap();
        assert!(tls.reload().is_err());
        assert!(tls_connect(addr, ca1.path()).await.is_ok());

        // Watch the files and swap in the second certificate.
        let stop = Arc::new(AtomicBool::new(false));
        std::fs::write(cert.path(), TEST_SERVER_CERT_PEM).unwrap();
        let watcher = tls.spawn_watcher(Duration::from_millis(50), Arc::clone(&stop));
        tokio::time::sleep(Duration::from_millis(20)).await;
        std::fs::write(cert.path(), TEST_SERVER_CERT2_PEM).unwrap();
        std::fs::write(key.path(), TEST_SERVER_KEY2_PEM).unwrap();

        let mut switched = false;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if tls_connect(addr, ca2.path()).await.is_ok() {
                switched = true;
                break;
            }
        }
        stop.store(true, Ordering::SeqCst);
        watcher.await.unwrap();
        assert!(switched, "the reloaded certificate was not served");
        assert!(tls_connect(addr, ca1.path()).await.is_err());

        // A connection made before the reload keeps working.
        assert!(matches!(
            send_recv(&mut old_conn, &Request::Ping).await,
            Response::Pong
        ));
    }

    // ── Follower writes, snapshot sync and promotion (bug/152) ──────────────

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
        let leader_server =
            TcpServer::new(ServerConfig::default()).with_replication(Arc::clone(&repl));
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
        let follower_server =
            TcpServer::with_graph_arc(ServerConfig::default(), Arc::clone(&graph))
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
}
