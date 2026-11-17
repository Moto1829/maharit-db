//! ワイヤプロトコルのメッセージ型（`Request`
//!  `Response`）。

use super::*;

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

pub(super) fn default_chunk_size() -> usize {
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
