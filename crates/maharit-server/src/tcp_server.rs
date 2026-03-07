//! TCP server for network-based query execution
//!
//! Provides:
//! - TCP connection handling with async I/O
//! - Length-prefixed message framing
//! - JSON request/response protocol
//! - Connection pool management
//! - Graceful shutdown

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use bytes::{Buf, BytesMut};
use maharit_core::{Graph, NodeId};
use maharit_query::{Executor, Parser, is_read_only};
use maharit_storage::TransactionManager;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, broadcast};
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
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:7687".to_string(),
            max_connections: 100,
            read_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(30),
        }
    }
}

/// Default chunk size for streaming results
pub const DEFAULT_CHUNK_SIZE: usize = 100;

/// Request message from client
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    /// Execute a query
    #[serde(rename = "query")]
    Query {
        query: String,
        /// Optional transaction ID for executing within a transaction
        #[serde(rename = "txId")]
        tx_id: Option<u64>,
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
    },

    /// Commit a transaction
    #[serde(rename = "commit")]
    Commit {
        #[serde(rename = "txId")]
        tx_id: u64,
    },

    /// Rollback a transaction
    #[serde(rename = "rollback")]
    Rollback {
        #[serde(rename = "txId")]
        tx_id: u64,
    },
}

fn default_chunk_size() -> usize {
    DEFAULT_CHUNK_SIZE
}

/// Response message to client
#[derive(Debug, Serialize)]
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
    graph: Arc<RwLock<Graph>>,
    stats: Arc<ServerStats>,
    shutdown: Arc<AtomicBool>,
    tx_manager: Arc<TransactionManager>,
    /// Optional leader replication manager: when set, write operations are
    /// automatically replicated to followers via WAL entries.
    replication: Option<Arc<LeaderReplicationManager>>,
}

impl TcpServer {
    /// Create a new TCP server
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            graph: Arc::new(RwLock::new(Graph::new())),
            stats: Arc::new(ServerStats::default()),
            shutdown: Arc::new(AtomicBool::new(false)),
            tx_manager: Arc::new(TransactionManager::new()),
            replication: None,
        }
    }

    /// Create a server with an existing graph
    pub fn with_graph(config: ServerConfig, graph: Graph) -> Self {
        Self {
            config,
            graph: Arc::new(RwLock::new(graph)),
            stats: Arc::new(ServerStats::default()),
            shutdown: Arc::new(AtomicBool::new(false)),
            tx_manager: Arc::new(TransactionManager::new()),
            replication: None,
        }
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
                    let config = self.config.clone();
                    let replication = self.replication.clone();
                    let mut shutdown_rx = shutdown_tx.subscribe();

                    tokio::spawn(async move {
                        let result = handle_connection(
                            socket,
                            graph,
                            stats.clone(),
                            shutdown,
                            tx_manager,
                            config,
                            replication,
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
    graph: Arc<RwLock<Graph>>,
    stats: Arc<ServerStats>,
    shutdown: Arc<AtomicBool>,
    tx_manager: Arc<TransactionManager>,
    config: ServerConfig,
    replication: Option<Arc<LeaderReplicationManager>>,
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
            Request::Query { query, tx_id: _ } => {
                stats.total_queries.fetch_add(1, Ordering::SeqCst);
                let span = tracing::info_span!("query", query = %query);
                let _enter = span.enter();
                let start = std::time::Instant::now();
                let resp = execute_query(&graph, &query, replication.as_deref()).await;
                tracing::info!(duration_us = start.elapsed().as_micros() as u64, "query completed");
                resp
            }
            Request::StreamQuery {
                query,
                tx_id: _,
                chunk_size,
            } => {
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
            Request::Ping => Response::Pong,
            Request::Stats => {
                let g = graph.read().await;
                Response::Stats {
                    connections: stats.current_connections.load(Ordering::SeqCst),
                    total_queries: stats.total_queries.load(Ordering::SeqCst),
                    nodes: g.node_count(),
                    edges: g.edge_count(),
                }
            }
            Request::Disconnect => {
                send_response(&mut socket, &Response::Goodbye, config.write_timeout).await?;
                break;
            }
            Request::BeginTransaction { read_only } => {
                let tx_id = if read_only {
                    tx_manager.begin_read_only()
                } else {
                    tx_manager.begin()
                };
                Response::TransactionBegun { tx_id }
            }
            Request::Commit { tx_id } => match tx_manager.commit(tx_id) {
                Ok(()) => Response::Committed { tx_id },
                Err(e) => Response::Error {
                    message: format!("Commit failed: {}", e),
                },
            },
            Request::Rollback { tx_id } => {
                let mut g = graph.write().await;
                match tx_manager.rollback(tx_id, &mut g) {
                    Ok(()) => Response::RolledBack { tx_id },
                    Err(e) => Response::Error {
                        message: format!("Rollback failed: {}", e),
                    },
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
async fn execute_streaming_query(
    socket: &mut TcpStream,
    graph: &Arc<RwLock<Graph>>,
    stats: &Arc<ServerStats>,
    query: &str,
    chunk_size: usize,
    write_timeout: Duration,
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

    // Log read-only classification (used for monitoring).
    let is_write = !is_read_only(&stmt);

    // Snapshot node/edge IDs before execution for WAL diff (only when needed).
    let (node_ids_before, edge_ids_before) = if is_write && replication.is_some() {
        let g = graph.read().await;
        (
            g.nodes().map(|n| n.id).collect::<HashSet<NodeId>>(),
            g.edges().map(|e| e.id).collect::<HashSet<u64>>(),
        )
    } else {
        (HashSet::new(), HashSet::new())
    };

    // For read-only queries use a shared read lock so multiple reads can run
    // concurrently.  Write queries still require an exclusive write lock.
    // Executors are scoped within braces so the raw pointer is dropped before
    // any `.await` point, keeping the future `Send`.
    let result = if is_write {
        let mut g = graph.write().await;
        let exec_result = {
            let mut executor = Executor::new(&mut g);
            executor.execute(stmt)
        }; // executor dropped here
        let r = match exec_result {
            Ok(r) => r,
            Err(e) => {
                let response = Response::Error {
                    message: format!("Execution error: {}", e),
                };
                return send_response(socket, &response, write_timeout).await;
            }
        };
        if let Some(repl) = replication {
            emit_wal_diff(&g, &node_ids_before, &edge_ids_before, repl).await;
        }
        r
    } else {
        let g = graph.read().await;
        // SAFETY: `is_read_only(&stmt)` returned true, so the executor will
        // not call any mutation methods on the graph.  The read lock guarantees
        // no concurrent writer can modify the graph while we execute.
        let exec_result = {
            let mut executor = unsafe { Executor::new_readonly(&g) };
            executor.execute(stmt)
        }; // executor (with *mut Graph) dropped before any .await
        match exec_result {
            Ok(r) => r,
            Err(e) => {
                let response = Response::Error {
                    message: format!("Execution error: {}", e),
                };
                return send_response(socket, &response, write_timeout).await;
            }
        }
    };

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

/// Execute a query and return the response.
///
/// The query is first parsed so that `is_read_only` can be evaluated before
/// acquiring any lock. Both paths currently use an exclusive write lock because
/// the `Executor` API requires `&mut Graph`. The lock-type split is in place so
/// that a future refactoring of `Executor` to accept `&Graph` for read-only
/// statements can adopt `graph.read()` without touching this call site.
async fn execute_query(
    graph: &Arc<RwLock<Graph>>,
    query: &str,
    replication: Option<&LeaderReplicationManager>,
) -> Response {
    // Parse the query first so we can inspect the AST before locking.
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

    // Detect write queries for WAL diff tracking.
    let is_write = !is_read_only(&stmt);

    // Snapshot node/edge IDs before execution for WAL diff (only when needed).
    let (node_ids_before, edge_ids_before) = if is_write && replication.is_some() {
        let g = graph.read().await;
        (
            g.nodes().map(|n| n.id).collect::<HashSet<NodeId>>(),
            g.edges().map(|e| e.id).collect::<HashSet<u64>>(),
        )
    } else {
        (HashSet::new(), HashSet::new())
    };

    // For read-only queries use a shared read lock so multiple reads can run
    // concurrently.  Write queries still require an exclusive write lock.
    // The executor is scoped so it is dropped before any `.await` point,
    // keeping the spawned future `Send`.
    let exec_result = if is_write {
        let mut g = graph.write().await;
        let result = {
            let mut executor = Executor::new(&mut g);
            executor.execute(stmt)
        }; // executor dropped here — no raw pointer across the await below
        if let (Ok(_), Some(repl)) = (&result, replication) {
            emit_wal_diff(&g, &node_ids_before, &edge_ids_before, repl).await;
        }
        result
    } else {
        let g = graph.read().await;
        // SAFETY: `is_read_only(&stmt)` returned true, so the executor will
        // not call any mutation methods on the graph.  The read lock guarantees
        // no concurrent writer can modify the graph while we execute.
        let result = {
            let mut executor = unsafe { Executor::new_readonly(&g) };
            executor.execute(stmt)
        }; // executor (containing *mut Graph) dropped before any .await
        result
    };

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
async fn emit_wal_diff(
    graph: &Graph,
    node_ids_before: &HashSet<NodeId>,
    edge_ids_before: &HashSet<u64>,
    replication: &LeaderReplicationManager,
) {
    // Detect new and deleted nodes.
    for node in graph.nodes() {
        if !node_ids_before.contains(&node.id) {
            replication
                .append_wal_entry(WalEntryData::CreateNode {
                    node_id: node.id,
                    labels: node.labels.clone(),
                })
                .await;
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
    for edge in graph.edges() {
        if !edge_ids_before.contains(&edge.id) {
            replication
                .append_wal_entry(WalEntryData::CreateEdge {
                    edge_id: edge.id,
                    from: edge.from,
                    to: edge.to,
                    label: edge.label.clone(),
                })
                .await;
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
            Request::Query { query, tx_id } => {
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
            Request::Query { query, tx_id } => {
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
            Request::BeginTransaction { read_only } => {
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
            Request::BeginTransaction { read_only } => {
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
            Request::Commit { tx_id } => {
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
            Request::Rollback { tx_id } => {
                assert_eq!(tx_id, 456);
            }
            _ => panic!("Expected Rollback request"),
        }
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
                chunk_size,
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
        let mut graph = Graph::new();
        let empty_nodes: HashSet<NodeId> = HashSet::new();
        let empty_edges: HashSet<u64> = HashSet::new();

        graph.create_node_with_labels(vec!["Person".to_string()]);

        emit_wal_diff(&graph, &empty_nodes, &empty_edges, &repl).await;

        // WAL LSN should be 1 after one CreateNode entry.
        assert_eq!(repl.get_stats().current_lsn, 1);
    }

    #[tokio::test]
    async fn test_emit_wal_diff_delete_node() {
        use crate::replication::{LeaderReplicationManager, ReplicationConfig};

        let repl = Arc::new(LeaderReplicationManager::new(ReplicationConfig::default()));
        let mut graph = Graph::new();

        // Simulate: node 0 existed before but no longer does.
        let before_nodes: HashSet<NodeId> = [0].iter().copied().collect();
        let empty_edges: HashSet<u64> = HashSet::new();

        emit_wal_diff(&graph, &before_nodes, &empty_edges, &repl).await;

        // One DeleteNode entry should have been emitted.
        assert_eq!(repl.get_stats().current_lsn, 1);
        drop(graph);
    }
}
