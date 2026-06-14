//! TCP server for network-based query execution
//!
//! Provides:
//! - TCP connection handling with async I/O
//! - Length-prefixed message framing
//! - JSON request/response protocol
//! - Connection pool management
//! - Graceful shutdown

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use bytes::{Buf, BytesMut};
use maharit_core::{ConcurrentGraph, ConstraintManager, EdgeId, FulltextManager, GraphBackend, NodeId, PropertyValue};
use maharit_query::{Executor, Parser, is_read_only};
use maharit_storage::TransactionManager;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::time::timeout;

use crate::replication::{LeaderReplicationManager, WalEntryData};
use crate::tracing_setup::TracingConfig;

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

/// `require_auth = true` のとき、リクエストの `sessionToken` を検証する。
///
/// - 認証が無効なら何もせず `None`
/// - トークン無し → `AuthError`
/// - トークンが無効/期限切れ → `AuthError`
/// - 有効 → `None`（呼び出し側はそのまま処理を続行）
fn check_session(
    require_auth: bool,
    auth: &Arc<Mutex<crate::auth::AuthManager>>,
    token: &Option<String>,
) -> Option<Response> {
    if !require_auth {
        return None;
    }
    let token_str = match token {
        Some(t) if !t.is_empty() => t.as_str(),
        _ => {
            return Some(Response::AuthError {
                message: "authentication required: missing sessionToken".to_string(),
            });
        }
    };
    let mut mgr = auth.lock().unwrap();
    match mgr.validate_session(token_str) {
        Ok(_) => None,
        Err(e) => Some(Response::AuthError {
            message: format!("invalid session: {}", e),
        }),
    }
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

fn default_chunk_size() -> usize {
    DEFAULT_CHUNK_SIZE
}

/// Response message to client
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    /// Query result
    #[serde(rename = "result")]
    Result { rows: Vec<HashMap<String, String>> },

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
        rows: Vec<HashMap<String, String>>,
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
    /// Shared constraint manager: persists across all query executions.
    constraints: Arc<Mutex<ConstraintManager>>,
    /// Shared fulltext index manager: persists across all query executions.
    fulltext: Arc<Mutex<FulltextManager>>,
    /// Optional leader replication manager: when set, write operations are
    /// automatically replicated to followers via WAL entries.
    replication: Option<Arc<LeaderReplicationManager>>,
    /// Authentication manager. Only enforced when `config.require_auth = true`.
    auth: Arc<Mutex<crate::auth::AuthManager>>,
}

impl TcpServer {
    /// Create a new TCP server
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            graph: Arc::new(ConcurrentGraph::new()),
            stats: Arc::new(ServerStats::default()),
            shutdown: Arc::new(AtomicBool::new(false)),
            tx_manager: Arc::new(TransactionManager::new()),
            constraints: Arc::new(Mutex::new(ConstraintManager::new())),
            fulltext: Arc::new(Mutex::new(FulltextManager::new())),
            replication: None,
            auth: Arc::new(Mutex::new(crate::auth::AuthManager::new())),
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
            constraints: Arc::new(Mutex::new(ConstraintManager::new())),
            fulltext: Arc::new(Mutex::new(FulltextManager::new())),
            replication: None,
            auth: Arc::new(Mutex::new(crate::auth::AuthManager::new())),
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
            constraints: Arc::new(Mutex::new(ConstraintManager::new())),
            fulltext: Arc::new(Mutex::new(FulltextManager::new())),
            replication: None,
            auth: Arc::new(Mutex::new(crate::auth::AuthManager::new())),
        }
    }

    /// Return a clone of the graph Arc
    pub fn graph_arc(&self) -> Arc<ConcurrentGraph> {
        Arc::clone(&self.graph)
    }

    /// Attach a leader replication manager.  Once attached, every successful
    /// write query automatically appends WAL entries to the manager, which
    /// broadcasts them to all connected followers.
    pub fn with_replication(mut self, manager: Arc<LeaderReplicationManager>) -> Self {
        self.replication = Some(manager);
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
                    let constraints = Arc::clone(&self.constraints);
                    let fulltext = Arc::clone(&self.fulltext);
                    let config = self.config.clone();
                    let replication = self.replication.clone();
                    let auth = Arc::clone(&self.auth);
                    let mut shutdown_rx = shutdown_tx.subscribe();

                    tokio::spawn(async move {
                        let result = handle_connection(
                            socket,
                            graph,
                            stats.clone(),
                            shutdown,
                            tx_manager,
                            constraints,
                            fulltext,
                            config,
                            replication,
                            auth,
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
    constraints: Arc<Mutex<ConstraintManager>>,
    fulltext: Arc<Mutex<FulltextManager>>,
    config: ServerConfig,
    replication: Option<Arc<LeaderReplicationManager>>,
    auth: Arc<Mutex<crate::auth::AuthManager>>,
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
            } => {
                if let Some(resp) = check_session(config.require_auth, &auth, &session_token) {
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
                                &constraints,
                                &fulltext,
                                replication.as_deref(),
                            )
                            .await
                        }
                        None => execute_query(&graph, &query, &constraints, &fulltext, replication.as_deref()).await,
                    };
                    tracing::info!(duration_us = start.elapsed().as_micros() as u64, "query completed");
                    resp
                }
            }
            Request::StreamQuery {
                query,
                tx_id: _,
                chunk_size,
                session_token,
            } => {
                if let Some(resp) = check_session(config.require_auth, &auth, &session_token) {
                    resp
                } else {
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
                        &constraints,
                        &fulltext,
                        replication.as_deref(),
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
            }
            Request::Ping => Response::Pong,
            Request::Stats => Response::Stats {
                connections: stats.current_connections.load(Ordering::SeqCst),
                total_queries: stats.total_queries.load(Ordering::SeqCst),
                nodes: graph.node_count(),
                edges: graph.edge_count(),
            },
            Request::Disconnect => {
                send_response(&mut socket, &Response::Goodbye, config.write_timeout).await?;
                break;
            }
            Request::BeginTransaction {
                read_only,
                session_token,
            } => {
                if let Some(resp) = check_session(config.require_auth, &auth, &session_token) {
                    resp
                } else {
                    let tx_id = if read_only {
                        tx_manager.begin_read_only()
                    } else {
                        tx_manager.begin()
                    };
                    Response::TransactionBegun { tx_id }
                }
            }
            Request::Commit {
                tx_id,
                session_token,
            } => {
                if let Some(resp) = check_session(config.require_auth, &auth, &session_token) {
                    resp
                } else {
                    match tx_manager.commit(tx_id) {
                        Ok(()) => Response::Committed { tx_id },
                        Err(e) => Response::Error {
                            message: format!("Commit failed: {}", e),
                        },
                    }
                }
            }
            Request::Rollback {
                tx_id,
                session_token,
            } => {
                if let Some(resp) = check_session(config.require_auth, &auth, &session_token) {
                    resp
                } else {
                    match tx_manager.rollback_concurrent(tx_id, &graph) {
                        Ok(()) => Response::RolledBack { tx_id },
                        Err(e) => Response::Error {
                            message: format!("Rollback failed: {}", e),
                        },
                    }
                }
            }
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
    constraints: &Arc<Mutex<ConstraintManager>>,
    fulltext: &Arc<Mutex<FulltextManager>>,
    replication: Option<&LeaderReplicationManager>,
) -> std::io::Result<()> {
    // Parse the query
    let stmt = match Parser::new(query) {
        Ok(mut parser) => match parser.parse() {
            Ok(stmt) => stmt,
            Err(e) => {
                let response = Response::Error {
                    message: format!("Parse error: {}", e),
                };
                return send_response(socket, &response, write_timeout).await;
            }
        },
        Err(e) => {
            let response = Response::Error {
                message: format!("Lexer error: {}", e),
            };
            return send_response(socket, &response, write_timeout).await;
        }
    };

    let is_write = !is_read_only(&stmt);

    // Snapshot node/edge IDs before execution for WAL diff (ConcurrentGraph: no lock needed).
    let (node_ids_before, edge_ids_before) = if is_write && replication.is_some() {
        (
            graph.node_ids().into_iter().collect::<HashSet<NodeId>>(),
            graph.edge_ids().into_iter().collect::<HashSet<EdgeId>>(),
        )
    } else {
        (HashSet::new(), HashSet::new())
    };

    // Clone shared managers into the executor for this query.
    let cm = constraints.lock().unwrap().clone();
    let fm = fulltext.lock().unwrap().clone();

    // SAFETY: ConcurrentGraph has interior mutability via DashMap; the executor
    // uses the raw pointer only during the synchronous execute() call below.
    let exec_result = {
        let mut executor = unsafe { Executor::new_concurrent_with_managers(graph, cm, fm) };
        let result = executor.execute(stmt);
        if result.is_ok() {
            let (new_cm, new_fm) = executor.into_managers();
            *constraints.lock().unwrap() = new_cm;
            *fulltext.lock().unwrap() = new_fm;
        }
        result
    };

    let result = match exec_result {
        Ok(r) => r,
        Err(e) => {
            let response = Response::Error {
                message: format!("Execution error: {}", e),
            };
            return send_response(socket, &response, write_timeout).await;
        }
    };

    if is_write
        && let Some(repl) = replication {
            emit_wal_diff(graph.as_ref(), &node_ids_before, &edge_ids_before, repl).await;
        }

    // Convert rows to HashMap format
    let all_rows: Vec<HashMap<String, String>> = result
        .rows
        .into_iter()
        .map(|row| {
            result
                .columns
                .iter()
                .zip(row.columns.iter())
                .map(|(col, val)| (col.clone(), val.to_string()))
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

type NodeSnapshot = (Vec<String>, Arc<HashMap<String, PropertyValue>>);
type EdgeSnapshot = (NodeId, NodeId, String, Arc<HashMap<String, PropertyValue>>);

/// Lightweight snapshot of ConcurrentGraph state captured before a write query.
///
/// `Arc`-cloned property maps act as copy-on-write snapshots: when the executor
/// modifies a node's properties via `Arc::make_mut`, a new map is allocated,
/// leaving the snapshot Arc pointing at the original unchanged data.
struct ConcurrentSnapshot {
    nodes: HashMap<NodeId, NodeSnapshot>,
    edges: HashMap<EdgeId, EdgeSnapshot>,
}

fn take_concurrent_snapshot(graph: &ConcurrentGraph) -> ConcurrentSnapshot {
    let nodes = graph
        .nodes()
        .map(|r| {
            let n = r.value();
            (n.id, (n.labels.clone(), Arc::clone(&n.properties)))
        })
        .collect();
    let edges = graph
        .edges()
        .map(|r| {
            let e = r.value();
            (e.id, (e.from, e.to, e.label.clone(), Arc::clone(&e.properties)))
        })
        .collect();
    ConcurrentSnapshot { nodes, edges }
}

/// Diff the graph against a pre-execution snapshot and record undo entries.
fn record_undo_diff_concurrent(
    graph: &ConcurrentGraph,
    snapshot: &ConcurrentSnapshot,
    tx_id: u64,
    tx_manager: &TransactionManager,
) {
    let current_node_ids: HashSet<NodeId> = graph.node_ids().into_iter().collect();
    let snap_node_ids: HashSet<NodeId> = snapshot.nodes.keys().copied().collect();

    for &id in current_node_ids.difference(&snap_node_ids) {
        let _ = tx_manager.record_node_created(tx_id, id);
    }
    for &id in snap_node_ids.difference(&current_node_ids) {
        let (labels, properties) = snapshot.nodes[&id].clone();
        let _ = tx_manager.record_node_deleted(tx_id, id, labels, properties);
    }
    for &id in snap_node_ids.intersection(&current_node_ids) {
        let (_, old_props) = &snapshot.nodes[&id];
        if let Some(node) = graph.get_node(id) {
            for (key, old_val) in old_props.iter() {
                if node.properties.get(key.as_str()) != Some(old_val) {
                    let _ = tx_manager.record_property_changed(
                        tx_id, id, key.clone(), Some(old_val.clone()),
                    );
                }
            }
            for key in node.properties.keys() {
                if !old_props.contains_key(key.as_str()) {
                    let _ = tx_manager.record_property_changed(tx_id, id, key.clone(), None);
                }
            }
        }
    }

    let current_edge_ids: HashSet<EdgeId> = graph.edge_ids().into_iter().collect();
    let snap_edge_ids: HashSet<EdgeId> = snapshot.edges.keys().copied().collect();

    for &id in current_edge_ids.difference(&snap_edge_ids) {
        let _ = tx_manager.record_edge_created(tx_id, id);
    }
    for &id in snap_edge_ids.difference(&current_edge_ids) {
        let (from, to, label, properties) = snapshot.edges[&id].clone();
        let _ = tx_manager.record_edge_deleted(tx_id, id, from, to, label, properties);
    }
    for &id in snap_edge_ids.intersection(&current_edge_ids) {
        let (_, _, _, old_props) = &snapshot.edges[&id];
        if let Some(edge) = graph.get_edge(id) {
            for (key, old_val) in old_props.iter() {
                if edge.properties.get(key.as_str()) != Some(old_val) {
                    let _ = tx_manager.record_edge_property_changed(
                        tx_id, id, key.clone(), Some(old_val.clone()),
                    );
                }
            }
            for key in edge.properties.keys() {
                if !old_props.contains_key(key.as_str()) {
                    let _ = tx_manager.record_edge_property_changed(tx_id, id, key.clone(), None);
                }
            }
        }
    }
}

/// Execute a write query within a transaction: snapshot → execute → record undo diff.
#[allow(clippy::too_many_arguments)]
async fn execute_query_with_tx(
    graph: &Arc<ConcurrentGraph>,
    query: &str,
    tx_id: u64,
    tx_manager: &TransactionManager,
    constraints: &Arc<Mutex<ConstraintManager>>,
    fulltext: &Arc<Mutex<FulltextManager>>,
    replication: Option<&LeaderReplicationManager>,
) -> Response {
    let stmt = match Parser::new(query) {
        Ok(mut p) => match p.parse() {
            Ok(s) => s,
            Err(e) => {
                return Response::Error {
                    message: format!("Parse error: {}", e),
                }
            }
        },
        Err(e) => {
            return Response::Error {
                message: format!("Lexer error: {}", e),
            }
        }
    };

    let is_write = !is_read_only(&stmt);

    if !is_write {
        return execute_query(graph, query, constraints, fulltext, replication).await;
    }

    // Snapshot before execution for undo tracking and WAL diff.
    let snapshot = take_concurrent_snapshot(graph);
    let (node_ids_before, edge_ids_before) = if replication.is_some() {
        (
            graph.node_ids().into_iter().collect::<HashSet<NodeId>>(),
            graph.edge_ids().into_iter().collect::<HashSet<EdgeId>>(),
        )
    } else {
        (HashSet::new(), HashSet::new())
    };

    // Clone shared managers into the executor for this query.
    let cm = constraints.lock().unwrap().clone();
    let fm = fulltext.lock().unwrap().clone();

    // SAFETY: ConcurrentGraph has interior mutability via DashMap.
    let exec_result = {
        let mut executor = unsafe { Executor::new_concurrent_with_managers(graph, cm, fm) };
        let result = executor.execute(stmt);
        if result.is_ok() {
            let (new_cm, new_fm) = executor.into_managers();
            *constraints.lock().unwrap() = new_cm;
            *fulltext.lock().unwrap() = new_fm;
        }
        result
    };

    match exec_result {
        Ok(result) => {
            record_undo_diff_concurrent(graph, &snapshot, tx_id, tx_manager);

            if let Some(repl) = replication {
                emit_wal_diff(graph.as_ref(), &node_ids_before, &edge_ids_before, repl).await;
            }

            let rows = result
                .rows
                .into_iter()
                .map(|row| {
                    result
                        .columns
                        .iter()
                        .zip(row.columns.iter())
                        .map(|(col, val)| (col.clone(), val.to_string()))
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
    constraints: &Arc<Mutex<ConstraintManager>>,
    fulltext: &Arc<Mutex<FulltextManager>>,
    replication: Option<&LeaderReplicationManager>,
) -> Response {
    let stmt = match Parser::new(query) {
        Ok(mut parser) => match parser.parse() {
            Ok(stmt) => stmt,
            Err(e) => {
                return Response::Error {
                    message: format!("Parse error: {}", e),
                };
            }
        },
        Err(e) => {
            return Response::Error {
                message: format!("Lexer error: {}", e),
            };
        }
    };

    let is_write = !is_read_only(&stmt);

    // Snapshot node/edge IDs before execution for WAL diff (no lock needed).
    let (node_ids_before, edge_ids_before) = if is_write && replication.is_some() {
        (
            graph.node_ids().into_iter().collect::<HashSet<NodeId>>(),
            graph.edge_ids().into_iter().collect::<HashSet<EdgeId>>(),
        )
    } else {
        (HashSet::new(), HashSet::new())
    };

    // Clone shared managers into the executor for this query.
    let cm = constraints.lock().unwrap().clone();
    let fm = fulltext.lock().unwrap().clone();

    // SAFETY: ConcurrentGraph has interior mutability via DashMap; the executor
    // uses the raw pointer only during the synchronous execute() call.
    let exec_result = {
        let mut executor = unsafe { Executor::new_concurrent_with_managers(graph, cm, fm) };
        let result = executor.execute(stmt);
        if result.is_ok() {
            let (new_cm, new_fm) = executor.into_managers();
            *constraints.lock().unwrap() = new_cm;
            *fulltext.lock().unwrap() = new_fm;
        }
        result
    };

    if is_write
        && let (Ok(_), Some(repl)) = (&exec_result, replication) {
            emit_wal_diff(graph.as_ref(), &node_ids_before, &edge_ids_before, repl).await;
        }

    match exec_result {
        Ok(result) => {
            let rows: Vec<HashMap<String, String>> = result
                .rows
                .into_iter()
                .map(|row| {
                    result
                        .columns
                        .iter()
                        .zip(row.columns.iter())
                        .map(|(col, val)| (col.clone(), val.to_string()))
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

/// Compute a diff of node/edge sets before vs after a write operation and emit
/// corresponding WAL entries to the replication manager.
///
/// Detects:
/// - New nodes (created): emits `WalEntryData::CreateNode`
/// - Deleted nodes: emits `WalEntryData::DeleteNode`
/// - New edges (created): emits `WalEntryData::CreateEdge`
/// - Deleted edges: emits `WalEntryData::DeleteEdge`
///
/// Property changes are not tracked automatically (a future improvement could
/// compare property maps before and after).
/// Serialize a `PropertyValue` to a JSON string for WAL transport.
fn property_value_to_wal_string(val: &PropertyValue) -> String {
    match val {
        PropertyValue::Null => "null".to_string(),
        PropertyValue::Bool(b) => b.to_string(),
        PropertyValue::Int(n) => n.to_string(),
        PropertyValue::Float(n) => n.to_string(),
        PropertyValue::String(s) => serde_json::to_string(s).unwrap_or_default(),
        other => serde_json::to_string(&other.to_string()).unwrap_or_default(),
    }
}

async fn emit_wal_diff(
    graph: &dyn GraphBackend,
    node_ids_before: &HashSet<NodeId>,
    edge_ids_before: &HashSet<EdgeId>,
    replication: &LeaderReplicationManager,
) {
    // Detect new and deleted nodes.
    for node in graph.all_nodes() {
        if !node_ids_before.contains(&node.id) {
            replication
                .append_wal_entry(WalEntryData::CreateNode {
                    node_id: node.id,
                    labels: node.labels.clone(),
                })
                .await;
            // Replicate properties of the new node.
            for (key, val) in node.properties.iter() {
                let value = property_value_to_wal_string(val);
                replication
                    .append_wal_entry(WalEntryData::SetProperty {
                        target_id: node.id,
                        is_node: true,
                        key: key.clone(),
                        value,
                    })
                    .await;
            }
        }
    }
    for &old_id in node_ids_before {
        if graph.get_node(old_id).is_none() {
            replication
                .append_wal_entry(WalEntryData::DeleteNode { node_id: old_id })
                .await;
        }
    }

    // Detect new and deleted edges.
    for edge in graph.all_edges() {
        if !edge_ids_before.contains(&edge.id) {
            replication
                .append_wal_entry(WalEntryData::CreateEdge {
                    edge_id: edge.id,
                    from: edge.from,
                    to: edge.to,
                    label: edge.label.clone(),
                })
                .await;
            // Replicate properties of the new edge.
            for (key, val) in edge.properties.iter() {
                let value = property_value_to_wal_string(val);
                replication
                    .append_wal_entry(WalEntryData::SetProperty {
                        target_id: edge.id,
                        is_node: false,
                        key: key.clone(),
                        value,
                    })
                    .await;
            }
        }
    }
    for &old_id in edge_ids_before {
        if graph.get_edge(old_id).is_none() {
            replication
                .append_wal_entry(WalEntryData::DeleteEdge { edge_id: old_id })
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // require_auth=false ならトークン無しでも None を返す
        assert!(check_session(false, &auth, &None).is_none());
        assert!(check_session(false, &auth, &Some("garbage".to_string())).is_none());
    }

    #[test]
    fn test_check_session_when_auth_enabled() {
        let auth = Arc::new(Mutex::new(crate::auth::AuthManager::new()));

        // require_auth=true でトークン無し → AuthError
        let resp = check_session(true, &auth, &None);
        assert!(matches!(resp, Some(Response::AuthError { .. })));

        // require_auth=true で無効なトークン → AuthError
        let resp = check_session(true, &auth, &Some("garbage".to_string()));
        assert!(matches!(resp, Some(Response::AuthError { .. })));

        // 正規ログイン後のトークンなら通る
        let token = {
            let mut mgr = auth.lock().unwrap();
            mgr.authenticate("admin", "admin").unwrap()
        };
        assert!(check_session(true, &auth, &Some(token)).is_none());
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
        row.insert("name".to_string(), "Alice".to_string());
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
                chunk_size, session_token: None,
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
        row.insert("name".to_string(), "Alice".to_string());
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
    async fn test_emit_wal_diff_create_node() {
        use crate::replication::{LeaderReplicationManager, ReplicationConfig};

        let repl = Arc::new(LeaderReplicationManager::new(ReplicationConfig::default()));
        let graph = ConcurrentGraph::new();
        let empty_nodes: HashSet<NodeId> = HashSet::new();
        let empty_edges: HashSet<EdgeId> = HashSet::new();

        graph.create_node_with_labels(vec!["Person".to_string()]);

        emit_wal_diff(&graph, &empty_nodes, &empty_edges, &repl).await;

        // WAL LSN should be 1 after one CreateNode entry.
        assert_eq!(repl.get_stats().current_lsn, 1);
    }

    #[tokio::test]
    async fn test_emit_wal_diff_delete_node() {
        use crate::replication::{LeaderReplicationManager, ReplicationConfig};

        let repl = Arc::new(LeaderReplicationManager::new(ReplicationConfig::default()));
        let graph = ConcurrentGraph::new();

        // Simulate: node 0 existed before but no longer does.
        let before_nodes: HashSet<NodeId> = [0].iter().copied().collect();
        let empty_edges: HashSet<EdgeId> = HashSet::new();

        emit_wal_diff(&graph, &before_nodes, &empty_edges, &repl).await;

        // One DeleteNode entry should have been emitted.
        assert_eq!(repl.get_stats().current_lsn, 1);
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
        assert!(matches!(resp, Response::Pong), "expected Pong, got {:?}", resp);
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
                tx_id: None, session_token: None,
            },
        )
        .await;
        assert!(matches!(resp, Response::Result { .. }), "CREATE failed: {:?}", resp);

        // MATCH
        let resp = send_recv(
            &mut stream,
            &Request::Query {
                query: "MATCH (n:Person {name: 'Alice'}) RETURN n.name".to_string(),
                tx_id: None, session_token: None,
            },
        )
        .await;
        match resp {
            Response::Result { rows } => {
                assert!(!rows.is_empty(), "expected at least one row");
                // String values are returned as JSON-quoted strings (e.g. `"Alice"`)
                assert_eq!(rows[0].get("n.name").map(String::as_str), Some("\"Alice\""));
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
                tx_id: None, session_token: None,
            },
        )
        .await;
        assert!(matches!(resp, Response::Result { .. }), "CREATE failed: {:?}", resp);

        // MATCH each property individually
        // String values are JSON-quoted; numeric/bool values are plain
        for (col, expected) in [
            ("n.s", "\"hello\""),
            ("n.i", "42"),
            ("n.b", "true"),
        ] {
            let resp = send_recv(
                &mut stream,
                &Request::Query {
                    query: format!("MATCH (n:Item) RETURN {}", col),
                    tx_id: None, session_token: None,
                },
            )
            .await;
            match resp {
                Response::Result { rows } => {
                    assert!(!rows.is_empty(), "no rows for {}", col);
                    assert_eq!(
                        rows[0].get(col).map(String::as_str),
                        Some(expected),
                        "mismatch for {}",
                        col
                    );
                }
                other => panic!("expected Result for {}, got {:?}", col, other),
            }
        }

        // Float property — just check it parses as a number
        let resp = send_recv(
            &mut stream,
            &Request::Query {
                query: "MATCH (n:Item) RETURN n.f".to_string(),
                tx_id: None, session_token: None,
            },
        )
        .await;
        match resp {
            Response::Result { rows } => {
                let val: f64 = rows[0]["n.f"].parse().expect("n.f should be a float");
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
                &Request::Query { query: q.to_string(), tx_id: None, session_token: None },
            )
            .await;
            assert!(
                matches!(resp, Response::Result { .. }),
                "setup query failed for '{}': {:?}", q, resp
            );
        }

        // Traverse: MATCH (a)-[:LINK]->(b) RETURN b.name
        let resp = send_recv(
            &mut stream,
            &Request::Query {
                query: "MATCH (a:A)-[:LINK]->(b:B) RETURN b.name".to_string(),
                tx_id: None, session_token: None,
            },
        )
        .await;
        match resp {
            Response::Result { rows } => {
                assert!(!rows.is_empty(), "traversal returned no rows");
                assert_eq!(rows[0].get("b.name").map(String::as_str), Some("\"dst\""));
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
                tx_id: None, session_token: None,
            },
        )
        .await;
        assert!(
            matches!(resp, Response::Error { .. }),
            "expected Error response, got {:?}", resp
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
                tx_id: None, session_token: None,
            },
        )
        .await;

        // Delete
        let resp = send_recv(
            &mut stream,
            &Request::Query {
                query: "MATCH (n:TmpNode {name: 'delete_me'}) DETACH DELETE n".to_string(),
                tx_id: None, session_token: None,
            },
        )
        .await;
        assert!(matches!(resp, Response::Result { .. }), "DELETE failed: {:?}", resp);

        // Verify gone
        let resp = send_recv(
            &mut stream,
            &Request::Query {
                query: "MATCH (n:TmpNode {name: 'delete_me'}) RETURN n.name".to_string(),
                tx_id: None, session_token: None,
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
                            tx_id: None, session_token: None,
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
                tx_id: None, session_token: None,
            },
        )
        .await;
        match resp {
            Response::Result { rows } => {
                assert_eq!(rows.len(), 10, "10件のConcWriteノードが存在するべき、実際: {}", rows.len());
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
                tx_id: None, session_token: None,
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
                        tx_id: None, session_token: None,
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
                        tx_id: None, session_token: None,
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
                    let stream = tokio::time::timeout(
                        Duration::from_secs(5),
                        TcpStream::connect(addr),
                    )
                    .await;

                    match stream {
                        Ok(Ok(mut s)) => {
                            // 接続できた場合は Ping を送って生存確認
                            let resp = send_recv(&mut s, &Request::Ping).await;
                            assert!(
                                matches!(resp, Response::Pong),
                                "client #{i} got unexpected response: {resp:?}"
                            );
                        }
                        // タイムアウトや接続拒否は許容（制限超過時の想定動作）
                        Ok(Err(_)) | Err(_) => {}
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
        assert!(matches!(resp, Response::Pong), "server should still respond after connection burst");
    }

    #[tokio::test]
    async fn test_transaction_isolation() {
        // 2つのクライアントが同時にトランザクションを開始し、
        // 互いにコミット前の変更が干渉しないことを確認
        let addr = start_test_server().await;

        let mut client_a = TcpStream::connect(addr).await.unwrap();
        let mut client_b = TcpStream::connect(addr).await.unwrap();

        // クライアントA: トランザクション開始
        let resp_a = send_recv(&mut client_a, &Request::BeginTransaction { read_only: false, session_token: None }).await;
        let tx_a = match resp_a {
            Response::TransactionBegun { tx_id } => tx_id,
            other => panic!("client A expected TransactionBegun, got {other:?}"),
        };

        // クライアントB: トランザクション開始
        let resp_b = send_recv(&mut client_b, &Request::BeginTransaction { read_only: false, session_token: None }).await;
        let tx_b = match resp_b {
            Response::TransactionBegun { tx_id } => tx_id,
            other => panic!("client B expected TransactionBegun, got {other:?}"),
        };

        // クライアントA: トランザクション内でノードを作成
        let resp = send_recv(
            &mut client_a,
            &Request::Query {
                query: "CREATE (n:TxIsolate {owner: 'A'}) RETURN n".to_string(),
                tx_id: Some(tx_a), session_token: None,
            },
        )
        .await;
        assert!(matches!(resp, Response::Result { .. }), "tx A CREATE failed: {resp:?}");

        // クライアントB: トランザクション内でノードを作成
        let resp = send_recv(
            &mut client_b,
            &Request::Query {
                query: "CREATE (n:TxIsolate {owner: 'B'}) RETURN n".to_string(),
                tx_id: Some(tx_b), session_token: None,
            },
        )
        .await;
        assert!(matches!(resp, Response::Result { .. }), "tx B CREATE failed: {resp:?}");

        // クライアントA: コミット
        let resp = send_recv(&mut client_a, &Request::Commit { tx_id: tx_a, session_token: None }).await;
        assert!(matches!(resp, Response::Committed { .. }), "tx A commit failed: {resp:?}");

        // クライアントB: ロールバック（Bの変更は取り消される）
        let resp = send_recv(&mut client_b, &Request::Rollback { tx_id: tx_b, session_token: None }).await;
        assert!(matches!(resp, Response::RolledBack { .. }), "tx B rollback failed: {resp:?}");

        // 確認: A のノードのみ残り、B のノードはロールバックされている
        let mut checker = TcpStream::connect(addr).await.unwrap();
        let resp = send_recv(
            &mut checker,
            &Request::Query {
                query: "MATCH (n:TxIsolate) RETURN n.owner".to_string(),
                tx_id: None, session_token: None,
            },
        )
        .await;
        match resp {
            Response::Result { rows } => {
                // A がコミット済みなので少なくとも1件、B はロールバック済みなので "B" は含まれない
                assert!(
                    rows.iter().any(|r| r.get("n.owner").map(|s| s.as_str()) == Some("\"A\"")),
                    "A's committed node should exist"
                );
                assert!(
                    !rows.iter().any(|r| r.get("n.owner").map(|s| s.as_str()) == Some("\"B\"")),
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
                    tx_id: None, session_token: None,
                },
            )
            .await;
            assert!(
                matches!(resp, Response::Result { .. }),
                "CREATE #{} failed: {:?}", i, resp
            );
        }

        // 同じ接続で MATCH して件数を確認
        let resp = send_recv(
            &mut stream,
            &Request::Query {
                query: "MATCH (n:Pipeline) RETURN n.idx".to_string(),
                tx_id: None, session_token: None,
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
                tx_id: None, session_token: None,
            },
        )
        .await;
        match resp {
            Response::Result { rows } => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].get("n.idx").map(String::as_str), Some("5"));
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
                query: "CREATE CONSTRAINT unique_test_id FOR (n:TestItem) REQUIRE n.id IS UNIQUE".to_string(),
                tx_id: None, session_token: None,
            },
        )
        .await;
        assert!(matches!(resp, Response::Result { .. }), "CREATE CONSTRAINT failed: {:?}", resp);

        // 最初のノード作成（成功するはず）
        let resp = send_recv(
            &mut stream,
            &Request::Query {
                query: "CREATE (:TestItem {id: 'item-1'})".to_string(),
                tx_id: None, session_token: None,
            },
        )
        .await;
        assert!(matches!(resp, Response::Result { .. }), "First CREATE failed: {:?}", resp);

        // 重複 id で2回目の作成（制約違反でエラーになるはず）
        let resp = send_recv(
            &mut stream,
            &Request::Query {
                query: "CREATE (:TestItem {id: 'item-1'})".to_string(),
                tx_id: None, session_token: None,
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
                tx_id: None, session_token: None,
            },
        )
        .await;
        match resp {
            Response::Result { rows } => {
                assert!(!rows.is_empty(), "SHOW CONSTRAINTS should return at least one row");
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
                tx_id: None, session_token: None,
            },
        )
        .await;
        assert!(matches!(resp, Response::Result { .. }), "CREATE FULLTEXT INDEX failed: {:?}", resp);

        // DROP FULLTEXT INDEX（永続化されていれば成功するはず）
        let resp = send_recv(
            &mut stream,
            &Request::Query {
                query: "DROP FULLTEXT INDEX ft_test_body".to_string(),
                tx_id: None, session_token: None,
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
