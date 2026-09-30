//! TCP server for network-based query execution
//!
//! Provides:
//! - TCP connection handling with async I/O
//! - Length-prefixed message framing
//! - JSON request/response protocol
//! - Connection pool management
//! - Graceful shutdown

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use bytes::{Buf, BytesMut};
use maharit_core::{
    ConcurrentGraph, ConstraintManager, FulltextManager, GraphBackend, PropertyIndex,
};
use maharit_query::{AstCache, Executor, Parser, is_read_only};
use maharit_storage::TransactionManager;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::time::timeout;

use crate::mutation_log::RecordingGraph;
use crate::replication::{
    FollowerReplicationManager, LeaderReplicationManager, ReplicationStats, WalEntryData,
};
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
    if !require_auth {
        return Ok(crate::auth::Role::Admin);
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
        Ok(session) => Ok(session.role),
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
) -> Option<Response> {
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
        return Some(Response::AuthError {
            message: "permission denied: user management requires admin role".to_string(),
        });
    }

    let mut mgr = auth.lock().unwrap();
    let resp = match stmt {
        Statement::CreateUser(cu) => match parse_role_str(&cu.role) {
            Some(r) => match mgr.create_user(&cu.username, &cu.password, r) {
                Ok(()) => result_message(format!(
                    "User '{}' created with role '{}'",
                    cu.username, cu.role
                )),
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            },
            None => Response::Error {
                message: format!("unknown role: '{}'", cu.role),
            },
        },
        Statement::DropUser(du) => match mgr.drop_user(&du.username) {
            Ok(()) => result_message(format!("User '{}' dropped", du.username)),
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
                Ok(()) => result_message(format!("User '{}' altered", au.username)),
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
    replication: Option<Arc<LeaderReplicationManager>>,
    /// Optional follower replication manager: when set, the `stats` response
    /// reports the follower's view of leader liveness.
    follower: Option<Arc<FollowerReplicationManager>>,
    /// Authentication manager. Only enforced when `config.require_auth = true`.
    auth: Arc<Mutex<crate::auth::AuthManager>>,
    /// Shared parsed-AST cache: avoids re-parsing identical query strings on the
    /// hot request path. Keyed by normalized query text.
    ast_cache: Arc<Mutex<AstCache>>,
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
    write_gate: tokio::sync::Mutex<()>,
}

impl SharedManagers {
    fn new() -> Self {
        Self {
            constraints: RwLock::new(ConstraintManager::new()),
            fulltext: RwLock::new(FulltextManager::new()),
            property_index: RwLock::new(PropertyIndex::new()),
            write_gate: tokio::sync::Mutex::new(()),
        }
    }

    /// Run `stmt` against `graph` with these managers.
    ///
    /// Write statements must be executed while holding `write_gate`. For writes
    /// with `record_wal`, the returned log holds every graph mutation in
    /// execution order (also when the statement fails midway, since the graph
    /// is not rolled back and followers must see the same state).
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
        let (result, wal, _) = self.run(graph, stmt, is_write, record_wal, false);
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
        record_wal: bool,
    ) -> (
        Result<maharit_query::ResultSet, maharit_query::ExecuteError>,
        Vec<WalEntryData>,
        Vec<UndoRecord>,
    ) {
        self.run(graph, stmt, true, record_wal, true)
    }

    fn run(
        &self,
        graph: &ConcurrentGraph,
        stmt: maharit_query::ast::Statement,
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
            let mut cm = self.constraints.write().unwrap_or_else(|e| e.into_inner());
            let mut fm = self.fulltext.write().unwrap_or_else(|e| e.into_inner());
            let mut pi = self
                .property_index
                .write()
                .unwrap_or_else(|e| e.into_inner());
            let mut recorder = RecordingGraph::new(graph, record_wal);
            if capture_undo {
                recorder = recorder.with_undo();
            }
            let result =
                Executor::new_with_backend_exclusive(&mut recorder, &mut cm, &mut fm, &mut pi)
                    .execute(stmt);
            let (wal, undo) = recorder.into_parts();
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
            (executor.execute(stmt), Vec::new(), Vec::new())
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
        for id in touched {
            pi.remove_node(id);
            fm.remove_node(id);
            if let Some(node) = graph.get_node(id) {
                let label = node.primary_label();
                for (key, value) in node.properties.iter() {
                    if pi.has_index(label, key) {
                        pi.index_property(id, key, value);
                    }
                }
                fm.index_node(id, label, &node.properties);
            }
        }
        Ok(recorder.into_log())
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
            replication: None,
            follower: None,
            auth: Arc::new(Mutex::new(crate::auth::AuthManager::new())),
            ast_cache: Arc::new(Mutex::new(AstCache::new(AST_CACHE_CAPACITY))),
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
            replication: None,
            follower: None,
            auth: Arc::new(Mutex::new(crate::auth::AuthManager::new())),
            ast_cache: Arc::new(Mutex::new(AstCache::new(AST_CACHE_CAPACITY))),
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
            replication: None,
            follower: None,
            auth: Arc::new(Mutex::new(crate::auth::AuthManager::new())),
            ast_cache: Arc::new(Mutex::new(AstCache::new(AST_CACHE_CAPACITY))),
        }
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

    /// Attach a leader replication manager.  Once attached, every successful
    /// write query automatically appends WAL entries to the manager, which
    /// broadcasts them to all connected followers.
    pub fn with_replication(mut self, manager: Arc<LeaderReplicationManager>) -> Self {
        self.replication = Some(manager);
        self
    }

    /// Attach a follower replication manager so the `stats` response can report
    /// this follower's view of leader liveness.
    pub fn with_follower(mut self, manager: Arc<FollowerReplicationManager>) -> Self {
        self.follower = Some(manager);
        self
    }

    /// Start the server
    pub async fn start(&self) -> std::io::Result<()> {
        // Initialise structured tracing (JSON to stderr; honours RUST_LOG env var)
        let _tracing_guard = TracingConfig::default().init();

        let listener = TcpListener::bind(&self.config.bind_address).await?;
        tracing::info!(address = %self.config.bind_address, "Server listening");
        println!("Server listening on {}", self.config.bind_address);

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

                    let graph = Arc::clone(&self.graph);
                    let stats = Arc::clone(&self.stats);
                    let shutdown = Arc::clone(&self.shutdown);
                    let tx_manager = Arc::clone(&self.tx_manager);
                    let managers = Arc::clone(&self.managers);
                    let config = self.config.clone();
                    let replication = self.replication.clone();
                    let follower = self.follower.clone();
                    let auth = Arc::clone(&self.auth);
                    let ast_cache = Arc::clone(&self.ast_cache);
                    let mut shutdown_rx = shutdown_tx.subscribe();

                    tokio::spawn(async move {
                        let result = handle_connection(
                            socket,
                            graph,
                            stats.clone(),
                            shutdown,
                            tx_manager,
                            managers,
                            config,
                            replication,
                            follower,
                            auth,
                            ast_cache,
                            &mut shutdown_rx,
                        )
                        .await;

                        if let Err(e) = result {
                            tracing::warn!(peer = %addr, error = %e, "Connection error");
                            eprintln!("Connection error from {}: {}", addr, e);
                        } else {
                            tracing::info!(peer = %addr, "Client disconnected");
                        }

                        stats.current_connections.fetch_sub(1, Ordering::SeqCst);
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

/// Handle a single client connection
#[allow(clippy::too_many_arguments)]
async fn handle_connection(
    mut socket: TcpStream,
    graph: Arc<ConcurrentGraph>,
    stats: Arc<ServerStats>,
    shutdown: Arc<AtomicBool>,
    tx_manager: Arc<TransactionManager>,
    managers: Arc<SharedManagers>,
    config: ServerConfig,
    replication: Option<Arc<LeaderReplicationManager>>,
    follower: Option<Arc<FollowerReplicationManager>>,
    auth: Arc<Mutex<crate::auth::AuthManager>>,
    ast_cache: Arc<Mutex<AstCache>>,
    shutdown_rx: &mut broadcast::Receiver<()>,
) -> std::io::Result<()> {
    let mut buffer = BytesMut::with_capacity(4096);

    loop {
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
        let response = match request {
            Request::Login { username, password } => {
                let mut mgr = auth.lock().unwrap();
                match mgr.authenticate(&username, &password) {
                    Ok(token) => {
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
                    Err(e) => Response::AuthError {
                        message: format!("{}", e),
                    },
                }
            }
            Request::Query {
                query,
                tx_id,
                session_token,
            } => match check_session(config.require_auth, &auth, &session_token) {
                Err(resp) => resp,
                Ok(role) => {
                    if let Some(resp) = try_handle_user_management(
                        &query,
                        &auth,
                        &ast_cache,
                        role,
                        config.require_auth,
                    ) {
                        resp
                    } else if let Some(resp) = authorize_query(role, &query) {
                        resp
                    } else {
                        stats.total_queries.fetch_add(1, Ordering::SeqCst);
                        let span = tracing::info_span!("query", query = %query);
                        let _enter = span.enter();
                        let start = std::time::Instant::now();
                        let resp = match tx_id {
                            Some(id) => {
                                execute_query_with_tx(
                                    &graph,
                                    &query,
                                    id,
                                    &tx_manager,
                                    &managers,
                                    replication.as_deref(),
                                    &ast_cache,
                                )
                                .await
                            }
                            None => {
                                execute_query(
                                    &graph,
                                    &query,
                                    &managers,
                                    replication.as_deref(),
                                    &ast_cache,
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
                }
            },
            Request::StreamQuery {
                query,
                tx_id: _,
                chunk_size,
                session_token,
            } => match check_session(config.require_auth, &auth, &session_token) {
                Err(resp) => resp,
                Ok(role) if authorize_query(role, &query).is_some() => {
                    authorize_query(role, &query).unwrap()
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
                        chunk_size,
                        config.write_timeout,
                        &managers,
                        replication.as_deref(),
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
                let replication_status = match follower.as_ref() {
                    Some(f) => Some(ReplicationStatus::from(f.get_stats())),
                    None => replication
                        .as_ref()
                        .map(|l| ReplicationStatus::from(l.get_stats())),
                };
                Response::Stats {
                    connections: stats.current_connections.load(Ordering::SeqCst),
                    total_queries: stats.total_queries.load(Ordering::SeqCst),
                    nodes: graph.node_count(),
                    edges: graph.edge_count(),
                    replication: replication_status,
                }
            }
            Request::Disconnect => {
                send_response(&mut socket, &Response::Goodbye, config.write_timeout).await?;
                break;
            }
            Request::BeginTransaction {
                read_only,
                session_token,
            } => match check_session(config.require_auth, &auth, &session_token) {
                Err(resp) => resp,
                Ok(role) => {
                    // 書き込みトランザクションの開始には Write 権限を要求する。
                    if !read_only
                        && crate::auth::AuthManager::check_role_permission(
                            role,
                            crate::auth::Operation::Write,
                        )
                        .is_err()
                    {
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
                    // Serialize with writers so the compensating WAL entries
                    // are ordered after the statements they undo.
                    let _write_guard = managers.write_gate.lock().await;
                    match managers.rollback(&graph, &tx_manager, tx_id, replication.is_some()) {
                        Ok(wal) => {
                            if let Some(repl) = replication.as_deref() {
                                emit_wal_entries(wal, repl).await;
                            }
                            Response::RolledBack { tx_id }
                        }
                        Err(e) => Response::Error {
                            message: format!("Rollback failed: {}", e),
                        },
                    }
                }
            },
        };

        send_response(&mut socket, &response, config.write_timeout).await?;
    }

    Ok(())
}

/// Read a length-prefixed message from the socket
async fn read_message(
    socket: &mut TcpStream,
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
async fn send_response(
    socket: &mut TcpStream,
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
async fn execute_streaming_query(
    socket: &mut TcpStream,
    graph: &Arc<ConcurrentGraph>,
    stats: &Arc<ServerStats>,
    query: &str,
    chunk_size: usize,
    write_timeout: Duration,
    managers: &SharedManagers,
    replication: Option<&LeaderReplicationManager>,
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

    let (exec_result, wal) = managers.execute(graph, stmt, is_write, replication.is_some());
    if let Some(repl) = replication {
        emit_wal_entries(wal, repl).await;
    }
    drop(write_guard);

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

// ── Transaction-aware query execution ────────────────────────────────────────

/// Execute a write query within a transaction: execute → record undo log → replicate.
#[allow(clippy::too_many_arguments)]
async fn execute_query_with_tx(
    graph: &Arc<ConcurrentGraph>,
    query: &str,
    tx_id: u64,
    tx_manager: &TransactionManager,
    managers: &SharedManagers,
    replication: Option<&LeaderReplicationManager>,
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
        return execute_query(graph, query, managers, replication, ast_cache).await;
    }

    // Serialize writers across execute → WAL emission (see SharedManagers).
    let _write_guard = managers.write_gate.lock().await;

    // Refuse to write under a transaction that is unknown or already
    // finished: its changes could never be rolled back.
    if !tx_manager.is_active(tx_id) {
        return Response::Error {
            message: format!("Execution error: transaction {} is not active", tx_id),
        };
    }

    let (exec_result, wal, undo) = managers.execute_in_tx(graph, stmt, replication.is_some());
    // Record undo even when the statement failed midway: its partial changes
    // were applied and must be reverted by ROLLBACK.
    if let Err(e) = tx_manager.record_undo(tx_id, undo) {
        tracing::error!(tx_id, error = %e, "failed to record undo log");
    }
    if let Some(repl) = replication {
        emit_wal_entries(wal, repl).await;
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
    managers: &SharedManagers,
    replication: Option<&LeaderReplicationManager>,
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

    let (exec_result, wal) = managers.execute(graph, stmt, is_write, replication.is_some());
    if let Some(repl) = replication {
        emit_wal_entries(wal, repl).await;
    }
    drop(write_guard);

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
        assert!(server.replication.is_some());
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
    async fn send_recv(stream: &mut TcpStream, req: &Request) -> Response {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
                tx_id: None,
                session_token: None,
            },
        )
        .await;
        let _ = send_recv(
            &mut stream,
            &Request::Query {
                query: "CREATE INDEX ON :Widget(id)".to_string(),
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
}
