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
use maharit_query::{AccessPolicy, AstCache, Executor, Parser, Value, is_read_only};
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

/// セッションに適用する ACL（認証が無効なとき、admin、規則が無いときは `None`）。
fn session_policy(
    auth: &Arc<Mutex<crate::auth::AuthManager>>,
    require_auth: bool,
    role: crate::auth::Role,
    user: &str,
) -> Option<AccessPolicy> {
    if !require_auth {
        return None;
    }
    auth.lock()
        .unwrap_or_else(|e| e.into_inner())
        .acl()
        .policy_for(user, role)
}

/// ACL で書き込みを拒否したエラーか。
fn is_access_denied(resp: &Response) -> bool {
    matches!(resp, Response::Error { message } if message.contains("access denied:"))
}

/// ACL 文の主体・対象・操作をサーバーの型に変換する。
fn acl_parts(
    st: &maharit_query::ast::AclStatement,
) -> Result<
    (
        crate::auth::AclSubject,
        crate::auth::AclResource,
        crate::auth::AclAction,
    ),
    Response,
> {
    use crate::auth::{AclAction, AclResource, AclSubject};
    use maharit_query::ast::{AclAction as A, AclGrantee, AclTarget};
    let subject = match &st.grantee {
        AclGrantee::User(u) => AclSubject::User(u.clone()),
        AclGrantee::Role(r) => match parse_role_str(r) {
            Some(role) => AclSubject::Role(role),
            None => {
                return Err(Response::Error {
                    message: format!("unknown role: '{}'", r),
                });
            }
        },
    };
    let resource = match &st.target {
        AclTarget::AllLabels => AclResource::AllLabels,
        AclTarget::Label(l) => AclResource::Label(l.clone()),
        AclTarget::Property(l, p) => AclResource::Property(l.clone(), p.clone()),
    };
    let action = match st.action {
        A::Read => AclAction::Read,
        A::Write => AclAction::Write,
    };
    Ok((subject, resource, action))
}

/// 規則を `GRANT READ ON :Label(prop) TO USER alice` の形で表す（監査ログと SHOW ACL 用）。
fn describe_acl(
    subject: &crate::auth::AclSubject,
    resource: &crate::auth::AclResource,
    action: crate::auth::AclAction,
) -> (String, String, String) {
    use crate::auth::{AclAction, AclResource, AclSubject};
    let subject = match subject {
        AclSubject::User(u) => format!("USER {}", u),
        AclSubject::Role(r) => format!("ROLE {}", role_label(r)),
    };
    let resource = match resource {
        AclResource::AllLabels => ":*".to_string(),
        AclResource::Label(l) => format!(":{}", l),
        AclResource::Property(l, p) => format!(":{}({})", l, p),
    };
    let action = match action {
        AclAction::Read => "READ",
        AclAction::Write => "WRITE",
    };
    (subject, resource, action.to_string())
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
            | Statement::Acl(_)
            | Statement::ShowAcl
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
        Statement::Acl(st) => {
            use maharit_query::ast::AclCommand;
            let (subject, resource, action) = match acl_parts(&st) {
                Ok(parts) => parts,
                Err(resp) => return Some(resp),
            };
            let (who, what, act) = describe_acl(&subject, &resource, action);
            let result = match st.command {
                AclCommand::Grant | AclCommand::Deny => {
                    let deny = st.command == AclCommand::Deny;
                    let rule = crate::auth::AclRule {
                        subject,
                        resource,
                        action,
                        permission: if deny {
                            crate::auth::AclPermission::Deny
                        } else {
                            crate::auth::AclPermission::Allow
                        },
                    };
                    let verb = if deny { "DENY" } else { "GRANT" };
                    mgr.add_acl_rule(rule)
                        .map(|()| format!("{} {} ON {} TO {}", verb, act, what, who))
                }
                AclCommand::Revoke => {
                    let n = mgr.revoke_acl(&subject, &resource, action);
                    Ok(format!(
                        "REVOKE {} ON {} FROM {}: {} rule(s) removed",
                        act, what, who, n
                    ))
                }
            };
            match result {
                Ok(msg) => {
                    audit_log(
                        audit,
                        AuthEvent::AclChanged {
                            rule: msg.clone(),
                            by: actor.to_string(),
                        },
                    );
                    result_message(msg)
                }
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
        Statement::ShowAcl => {
            let rows = mgr
                .acl()
                .rules()
                .iter()
                .map(|r| {
                    let (who, what, act) = describe_acl(&r.subject, &r.resource, r.action);
                    let permission = match r.permission {
                        crate::auth::AclPermission::Allow => "GRANT",
                        crate::auth::AclPermission::Deny => "DENY",
                    };
                    HashMap::from([
                        ("subject".to_string(), serde_json::Value::String(who)),
                        ("resource".to_string(), serde_json::Value::String(what)),
                        ("action".to_string(), serde_json::Value::String(act)),
                        (
                            "permission".to_string(),
                            serde_json::Value::String(permission.to_string()),
                        ),
                    ])
                })
                .collect();
            Response::Result { rows }
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
        self.execute_with_params(graph, stmt, HashMap::new(), is_write, record_wal, None)
    }

    /// [`execute`](Self::execute) with `$name` query parameters.
    fn execute_with_params(
        &self,
        graph: &ConcurrentGraph,
        stmt: maharit_query::ast::Statement,
        params: HashMap<String, Value>,
        is_write: bool,
        record_wal: bool,
        policy: Option<AccessPolicy>,
    ) -> (
        Result<maharit_query::ResultSet, maharit_query::ExecuteError>,
        Vec<WalEntryData>,
    ) {
        let (result, wal, _) = self.run(graph, stmt, params, is_write, record_wal, false, policy);
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
        policy: Option<AccessPolicy>,
    ) -> (
        Result<maharit_query::ResultSet, maharit_query::ExecuteError>,
        Vec<WalEntryData>,
        Vec<UndoRecord>,
    ) {
        self.run(graph, stmt, params, true, record_wal, true, policy)
    }

    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        graph: &ConcurrentGraph,
        stmt: maharit_query::ast::Statement,
        params: HashMap<String, Value>,
        is_write: bool,
        record_wal: bool,
        capture_undo: bool,
        policy: Option<AccessPolicy>,
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
            // スキーマ変更（索引・制約）は ACL を通さずに全ノードを対象にする
            let policy = if is_ddl { None } else { policy };
            let result =
                Executor::new_with_backend_exclusive(&mut recorder, &mut cm, &mut fm, &mut pi)
                    .with_access_policy(policy)
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
            let mut executor = unsafe { Executor::new_concurrent_shared(graph, &cm, &fm, &pi) }
                .with_access_policy(policy);
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
                            let policy = session_policy(&auth, config.require_auth, role, &user);
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
                                            policy,
                                        )
                                        .await
                                    }
                                    None => {
                                        execute_query(
                                            &graph, &query, params, &managers, sinks, &ast_cache,
                                            policy,
                                        )
                                        .await
                                    }
                                };
                                if is_access_denied(&resp) {
                                    audit_denied(audit, &user, "acl_write");
                                }
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
                    Ok((role, user)) => {
                        stats.total_queries.fetch_add(1, Ordering::SeqCst);
                        tracing::info!(query = %query, "streaming query");
                        let policy = session_policy(&auth, config.require_auth, role, &user);
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
                            policy,
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
    managers: &Arc<SharedManagers>,
    sinks: WriteSinks<'_>,
    ast_cache: &Arc<Mutex<AstCache>>,
    policy: Option<AccessPolicy>,
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

    let record = sinks.record();
    let (exec_result, wal) = run_statement(graph, managers, move |graph, managers| {
        managers.execute_with_params(graph, stmt, params, is_write, record, policy)
    })
    .await;
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

/// Run a statement on the blocking thread pool and wait for it.
///
/// Statements run synchronously and may take seconds. Running them on the async
/// worker threads would stall every other connection (queries, pings, new
/// connections, replication heartbeats) once as many heavy statements as there
/// are workers are in flight, so they get their own threads instead.
///
/// A writer keeps holding `write_gate` while it waits here, so writes are still
/// applied one at a time and in the order their WAL entries are published.
async fn run_statement<R, F>(
    graph: &Arc<ConcurrentGraph>,
    managers: &Arc<SharedManagers>,
    f: F,
) -> R
where
    R: Send + 'static,
    F: FnOnce(&ConcurrentGraph, &SharedManagers) -> R + Send + 'static,
{
    let graph = Arc::clone(graph);
    let managers = Arc::clone(managers);
    match tokio::task::spawn_blocking(move || f(&graph, &managers)).await {
        Ok(result) => result,
        // The statement panicked: keep the panic instead of inventing a result.
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
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
    managers: &Arc<SharedManagers>,
    sinks: WriteSinks<'_>,
    ast_cache: &Arc<Mutex<AstCache>>,
    policy: Option<AccessPolicy>,
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
        return execute_query(graph, query, params, managers, sinks, ast_cache, policy).await;
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

    let record = sinks.record();
    let (exec_result, wal, undo) = run_statement(graph, managers, move |graph, managers| {
        managers.execute_in_tx(graph, stmt, params, record, policy)
    })
    .await;
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
    managers: &Arc<SharedManagers>,
    sinks: WriteSinks<'_>,
    ast_cache: &Arc<Mutex<AstCache>>,
    policy: Option<AccessPolicy>,
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

    let record = sinks.record();
    let (exec_result, wal) = run_statement(graph, managers, move |graph, managers| {
        managers.execute_with_params(graph, stmt, params, is_write, record, policy)
    })
    .await;
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
mod tests;
