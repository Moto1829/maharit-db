//! MaharitDB Client Library
//!
//! A client library for connecting to MaharitDB TCP server.
//!
//! # Example
//!
//! ```ignore
//! use maharit_client::Client;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut client = Client::connect("localhost:7687").await?;
//!
//!     // Create a node
//!     client.execute("CREATE (n:Person {name: \"Alice\"})").await?;
//!
//!     // Query nodes
//!     let result = client.query("MATCH (n:Person) RETURN n.name").await?;
//!     for row in &result.rows {
//!         println!("{:?}", row);
//!     }
//!
//!     Ok(())
//! }
//! ```

use std::collections::HashMap;
use std::time::Duration;

use bytes::{Buf, BytesMut};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Client errors
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("connection error: {0}")]
    Connection(#[from] std::io::Error),

    #[error("server error: {0}")]
    Server(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("connection timeout")]
    Timeout,

    #[error("connection closed")]
    ConnectionClosed,
}

pub type Result<T> = std::result::Result<T, ClientError>;

/// Client configuration
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Connection timeout
    pub connect_timeout: Duration,
    /// Read timeout
    pub read_timeout: Duration,
    /// Write timeout
    pub write_timeout: Duration,
    /// Enable auto-reconnect on connection loss
    pub auto_reconnect: bool,
    /// Maximum number of reconnection attempts
    pub max_reconnect_attempts: u32,
    /// Delay between reconnection attempts
    pub reconnect_delay: Duration,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(30),
            auto_reconnect: false,
            max_reconnect_attempts: 3,
            reconnect_delay: Duration::from_millis(500),
        }
    }
}

/// Transaction ID
pub type TxId = u64;

/// Default chunk size for streaming
pub const DEFAULT_CHUNK_SIZE: usize = 100;

/// Request message to server
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum Request {
    #[serde(rename = "query")]
    Query {
        query: String,
        #[serde(rename = "txId", skip_serializing_if = "Option::is_none")]
        tx_id: Option<TxId>,
    },

    #[serde(rename = "streamQuery")]
    StreamQuery {
        query: String,
        #[serde(rename = "txId", skip_serializing_if = "Option::is_none")]
        tx_id: Option<TxId>,
        #[serde(rename = "chunkSize")]
        chunk_size: usize,
    },

    #[serde(rename = "ping")]
    Ping,

    #[serde(rename = "stats")]
    Stats,

    #[serde(rename = "disconnect")]
    Disconnect,

    #[serde(rename = "begin")]
    BeginTransaction {
        #[serde(rename = "readOnly")]
        read_only: bool,
    },

    #[serde(rename = "commit")]
    Commit {
        #[serde(rename = "txId")]
        tx_id: TxId,
    },

    #[serde(rename = "rollback")]
    Rollback {
        #[serde(rename = "txId")]
        tx_id: TxId,
    },
}

/// Response message from server
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Response {
    #[serde(rename = "result")]
    Result { rows: Vec<HashMap<String, String>> },

    #[serde(rename = "error")]
    Error { message: String },

    #[serde(rename = "pong")]
    Pong,

    #[serde(rename = "stats")]
    Stats {
        connections: u64,
        total_queries: u64,
        nodes: usize,
        edges: usize,
    },

    #[serde(rename = "goodbye")]
    Goodbye,

    #[serde(rename = "transactionBegun")]
    TransactionBegun {
        #[serde(rename = "txId")]
        tx_id: TxId,
    },

    #[serde(rename = "committed")]
    Committed {
        #[serde(rename = "txId")]
        tx_id: TxId,
    },

    #[serde(rename = "rolledBack")]
    RolledBack {
        #[serde(rename = "txId")]
        tx_id: TxId,
    },

    #[serde(rename = "streamStart")]
    StreamStart {
        #[serde(rename = "streamId")]
        stream_id: u64,
        #[serde(rename = "totalRows")]
        total_rows: Option<usize>,
    },

    #[serde(rename = "streamChunk")]
    StreamChunk {
        #[serde(rename = "streamId")]
        stream_id: u64,
        #[serde(rename = "chunkIndex")]
        chunk_index: usize,
        rows: Vec<HashMap<String, String>>,
    },

    #[serde(rename = "streamEnd")]
    StreamEnd {
        #[serde(rename = "streamId")]
        stream_id: u64,
        #[serde(rename = "totalRows")]
        total_rows: usize,
    },
}

/// Stream ID type
pub type StreamId = u64;

/// Streaming result that allows iterating over chunks
pub struct StreamingResult<'a> {
    client: &'a mut Client,
    stream_id: StreamId,
    total_rows: Option<usize>,
    rows_received: usize,
    finished: bool,
}

impl<'a> StreamingResult<'a> {
    /// Get the stream ID
    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// Get the total number of rows (if known)
    pub fn total_rows(&self) -> Option<usize> {
        self.total_rows
    }

    /// Get the number of rows received so far
    pub fn rows_received(&self) -> usize {
        self.rows_received
    }

    /// Check if the stream has finished
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Get the next chunk of rows
    pub async fn next_chunk(&mut self) -> Result<Option<Vec<HashMap<String, String>>>> {
        if self.finished {
            return Ok(None);
        }

        match self.client.receive_response().await? {
            Response::StreamChunk {
                stream_id,
                chunk_index: _,
                rows,
            } => {
                if stream_id != self.stream_id {
                    return Err(ClientError::Protocol(format!(
                        "stream ID mismatch: expected {}, got {}",
                        self.stream_id, stream_id
                    )));
                }
                self.rows_received += rows.len();
                Ok(Some(rows))
            }
            Response::StreamEnd {
                stream_id,
                total_rows,
            } => {
                if stream_id != self.stream_id {
                    return Err(ClientError::Protocol(format!(
                        "stream ID mismatch: expected {}, got {}",
                        self.stream_id, stream_id
                    )));
                }
                self.rows_received = total_rows;
                self.finished = true;
                Ok(None)
            }
            Response::Error { message } => Err(ClientError::Server(message)),
            _ => Err(ClientError::Protocol(
                "unexpected response during streaming".to_string(),
            )),
        }
    }

    /// Collect all remaining rows into a single vector
    pub async fn collect_all(mut self) -> Result<Vec<HashMap<String, String>>> {
        let mut all_rows = Vec::new();
        while let Some(chunk) = self.next_chunk().await? {
            all_rows.extend(chunk);
        }
        Ok(all_rows)
    }
}

/// Query result
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// Result rows
    pub rows: Vec<HashMap<String, String>>,
}

impl QueryResult {
    /// Get the number of rows
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Check if result is empty
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Get a single value from the first row
    pub fn get_single(&self, column: &str) -> Option<&String> {
        self.rows.first().and_then(|row| row.get(column))
    }
}

/// Server statistics
#[derive(Debug, Clone)]
pub struct ServerStats {
    /// Current number of connections
    pub connections: u64,
    /// Total queries executed
    pub total_queries: u64,
    /// Number of nodes in the graph
    pub nodes: usize,
    /// Number of edges in the graph
    pub edges: usize,
}

/// MaharitDB client
pub struct Client {
    stream: TcpStream,
    config: ClientConfig,
    buffer: BytesMut,
    addr: String,
}

impl Client {
    /// Connect to a MaharitDB server with default configuration
    pub async fn connect(addr: &str) -> Result<Self> {
        Self::connect_with_config(addr, ClientConfig::default()).await
    }

    /// Connect to a MaharitDB server with custom configuration
    pub async fn connect_with_config(addr: &str, config: ClientConfig) -> Result<Self> {
        let stream = timeout(config.connect_timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| ClientError::Timeout)?
            .map_err(ClientError::Connection)?;

        Ok(Self {
            stream,
            config,
            buffer: BytesMut::with_capacity(4096),
            addr: addr.to_string(),
        })
    }

    /// Attempt to reconnect to the server
    pub async fn reconnect(&mut self) -> Result<()> {
        let stream = timeout(self.config.connect_timeout, TcpStream::connect(&self.addr))
            .await
            .map_err(|_| ClientError::Timeout)?
            .map_err(ClientError::Connection)?;

        self.stream = stream;
        self.buffer.clear();
        Ok(())
    }

    /// Try to reconnect with retries
    async fn try_reconnect(&mut self) -> bool {
        for attempt in 0..self.config.max_reconnect_attempts {
            if attempt > 0 {
                // Exponential backoff
                tokio::time::sleep(self.config.reconnect_delay * attempt).await;
            }

            if self.reconnect().await.is_ok() {
                return true;
            }
        }
        false
    }

    /// Check if an error should trigger reconnection
    fn should_reconnect(err: &ClientError) -> bool {
        matches!(err, ClientError::ConnectionClosed | ClientError::Connection(_))
    }

    /// Execute a query and return the result
    pub async fn query(&mut self, query: &str) -> Result<QueryResult> {
        self.query_in_tx(query, None).await
    }

    /// Execute a query within a transaction
    pub async fn query_in_tx(&mut self, query: &str, tx_id: Option<TxId>) -> Result<QueryResult> {
        let result = self.query_internal(query, tx_id).await;

        // Only auto-reconnect if not in a transaction
        if result.is_err() && self.config.auto_reconnect && tx_id.is_none() {
            if Self::should_reconnect(result.as_ref().unwrap_err()) {
                if self.try_reconnect().await {
                    return self.query_internal(query, tx_id).await;
                }
            }
        }

        result
    }

    async fn query_internal(&mut self, query: &str, tx_id: Option<TxId>) -> Result<QueryResult> {
        let request = Request::Query {
            query: query.to_string(),
            tx_id,
        };

        self.send_request(&request).await?;

        match self.receive_response().await? {
            Response::Result { rows } => Ok(QueryResult { rows }),
            Response::Error { message } => Err(ClientError::Server(message)),
            _ => Err(ClientError::Protocol(
                "unexpected response type".to_string(),
            )),
        }
    }

    /// Execute a query without returning results (for CREATE, DELETE, etc.)
    pub async fn execute(&mut self, query: &str) -> Result<()> {
        self.query(query).await?;
        Ok(())
    }

    /// Execute a query with streaming results
    ///
    /// Returns a `StreamingResult` that allows iterating over chunks of rows.
    /// This is useful for large result sets that shouldn't be loaded entirely into memory.
    ///
    /// # Example
    /// ```ignore
    /// let mut stream = client.query_stream("MATCH (n) RETURN n", 50).await?;
    /// while let Some(chunk) = stream.next_chunk().await? {
    ///     for row in chunk {
    ///         println!("{:?}", row);
    ///     }
    /// }
    /// ```
    pub async fn query_stream(&mut self, query: &str, chunk_size: usize) -> Result<StreamingResult<'_>> {
        self.query_stream_in_tx(query, None, chunk_size).await
    }

    /// Execute a streaming query within a transaction
    pub async fn query_stream_in_tx(
        &mut self,
        query: &str,
        tx_id: Option<TxId>,
        chunk_size: usize,
    ) -> Result<StreamingResult<'_>> {
        let request = Request::StreamQuery {
            query: query.to_string(),
            tx_id,
            chunk_size: if chunk_size == 0 { DEFAULT_CHUNK_SIZE } else { chunk_size },
        };

        self.send_request(&request).await?;

        // Wait for StreamStart response
        match self.receive_response().await? {
            Response::StreamStart {
                stream_id,
                total_rows,
            } => Ok(StreamingResult {
                client: self,
                stream_id,
                total_rows,
                rows_received: 0,
                finished: false,
            }),
            Response::Error { message } => Err(ClientError::Server(message)),
            _ => Err(ClientError::Protocol(
                "expected StreamStart response".to_string(),
            )),
        }
    }

    /// Execute a query within a transaction without returning results
    pub async fn execute_in_tx(&mut self, query: &str, tx_id: TxId) -> Result<()> {
        self.query_in_tx(query, Some(tx_id)).await?;
        Ok(())
    }

    /// Begin a new transaction
    pub async fn begin(&mut self) -> Result<TxId> {
        self.begin_with_options(false).await
    }

    /// Begin a read-only transaction
    pub async fn begin_read_only(&mut self) -> Result<TxId> {
        self.begin_with_options(true).await
    }

    /// Begin a transaction with options
    async fn begin_with_options(&mut self, read_only: bool) -> Result<TxId> {
        let request = Request::BeginTransaction { read_only };
        self.send_request(&request).await?;

        match self.receive_response().await? {
            Response::TransactionBegun { tx_id } => Ok(tx_id),
            Response::Error { message } => Err(ClientError::Server(message)),
            _ => Err(ClientError::Protocol(
                "expected transaction begun response".to_string(),
            )),
        }
    }

    /// Commit a transaction
    pub async fn commit(&mut self, tx_id: TxId) -> Result<()> {
        let request = Request::Commit { tx_id };
        self.send_request(&request).await?;

        match self.receive_response().await? {
            Response::Committed { tx_id: _ } => Ok(()),
            Response::Error { message } => Err(ClientError::Server(message)),
            _ => Err(ClientError::Protocol(
                "expected committed response".to_string(),
            )),
        }
    }

    /// Rollback a transaction
    pub async fn rollback(&mut self, tx_id: TxId) -> Result<()> {
        let request = Request::Rollback { tx_id };
        self.send_request(&request).await?;

        match self.receive_response().await? {
            Response::RolledBack { tx_id: _ } => Ok(()),
            Response::Error { message } => Err(ClientError::Server(message)),
            _ => Err(ClientError::Protocol(
                "expected rolled back response".to_string(),
            )),
        }
    }

    /// Ping the server to check connectivity
    pub async fn ping(&mut self) -> Result<()> {
        let result = self.ping_internal().await;

        if result.is_err() && self.config.auto_reconnect {
            if Self::should_reconnect(result.as_ref().unwrap_err()) {
                if self.try_reconnect().await {
                    return self.ping_internal().await;
                }
            }
        }

        result
    }

    async fn ping_internal(&mut self) -> Result<()> {
        self.send_request(&Request::Ping).await?;

        match self.receive_response().await? {
            Response::Pong => Ok(()),
            Response::Error { message } => Err(ClientError::Server(message)),
            _ => Err(ClientError::Protocol("expected pong response".to_string())),
        }
    }

    /// Get server statistics
    pub async fn stats(&mut self) -> Result<ServerStats> {
        self.send_request(&Request::Stats).await?;

        match self.receive_response().await? {
            Response::Stats {
                connections,
                total_queries,
                nodes,
                edges,
            } => Ok(ServerStats {
                connections,
                total_queries,
                nodes,
                edges,
            }),
            Response::Error { message } => Err(ClientError::Server(message)),
            _ => Err(ClientError::Protocol("expected stats response".to_string())),
        }
    }

    /// Gracefully disconnect from the server
    pub async fn disconnect(mut self) -> Result<()> {
        self.send_request(&Request::Disconnect).await?;

        match self.receive_response().await? {
            Response::Goodbye => Ok(()),
            _ => Ok(()), // Accept any response when disconnecting
        }
    }

    /// Check if the connection is alive
    pub async fn is_alive(&mut self) -> bool {
        self.ping().await.is_ok()
    }

    /// Get the server address
    pub fn addr(&self) -> &str {
        &self.addr
    }

    // Internal: send a request to the server
    async fn send_request(&mut self, request: &Request) -> Result<()> {
        let json = serde_json::to_vec(request)?;
        let len = json.len() as u32;

        let write_future = async {
            self.stream.write_all(&len.to_be_bytes()).await?;
            self.stream.write_all(&json).await?;
            self.stream.flush().await
        };

        timeout(self.config.write_timeout, write_future)
            .await
            .map_err(|_| ClientError::Timeout)?
            .map_err(ClientError::Connection)
    }

    // Internal: receive a response from the server
    async fn receive_response(&mut self) -> Result<Response> {
        // Read message length
        loop {
            if self.buffer.len() >= 4 {
                let len = u32::from_be_bytes([
                    self.buffer[0],
                    self.buffer[1],
                    self.buffer[2],
                    self.buffer[3],
                ]) as usize;

                if self.buffer.len() >= 4 + len {
                    self.buffer.advance(4);
                    let message = self.buffer.split_to(len);
                    let response: Response = serde_json::from_slice(&message)?;
                    return Ok(response);
                }
            }

            // Read more data
            let read_future = self.stream.read_buf(&mut self.buffer);
            let n = timeout(self.config.read_timeout, read_future)
                .await
                .map_err(|_| ClientError::Timeout)?
                .map_err(ClientError::Connection)?;

            if n == 0 {
                return Err(ClientError::ConnectionClosed);
            }
        }
    }
}

/// Connection pool for managing multiple client connections
pub mod pool {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::{Mutex, Semaphore};

    /// Connection pool configuration
    #[derive(Debug, Clone)]
    pub struct PoolConfig {
        /// Minimum number of connections in the pool
        pub min_connections: usize,
        /// Maximum number of connections in the pool
        pub max_connections: usize,
        /// Client configuration for each connection
        pub client_config: ClientConfig,
        /// Connection idle timeout (connections idle longer will be closed)
        pub idle_timeout: Duration,
    }

    impl Default for PoolConfig {
        fn default() -> Self {
            Self {
                min_connections: 1,
                max_connections: 10,
                client_config: ClientConfig::default(),
                idle_timeout: Duration::from_secs(300),
            }
        }
    }

    /// A pooled connection that returns to the pool when dropped
    pub struct PooledConnection {
        client: Option<Client>,
        pool: Arc<ConnectionPoolInner>,
    }

    impl PooledConnection {
        /// Execute a query and return the result
        pub async fn query(&mut self, query: &str) -> Result<QueryResult> {
            self.client.as_mut().unwrap().query(query).await
        }

        /// Execute a query without returning results
        pub async fn execute(&mut self, query: &str) -> Result<()> {
            self.client.as_mut().unwrap().execute(query).await
        }

        /// Ping the server
        pub async fn ping(&mut self) -> Result<()> {
            self.client.as_mut().unwrap().ping().await
        }

        /// Get server statistics
        pub async fn stats(&mut self) -> Result<ServerStats> {
            self.client.as_mut().unwrap().stats().await
        }

        /// Begin a new transaction
        pub async fn begin(&mut self) -> Result<TxId> {
            self.client.as_mut().unwrap().begin().await
        }

        /// Commit a transaction
        pub async fn commit(&mut self, tx_id: TxId) -> Result<()> {
            self.client.as_mut().unwrap().commit(tx_id).await
        }

        /// Rollback a transaction
        pub async fn rollback(&mut self, tx_id: TxId) -> Result<()> {
            self.client.as_mut().unwrap().rollback(tx_id).await
        }
    }

    impl Drop for PooledConnection {
        fn drop(&mut self) {
            if let Some(client) = self.client.take() {
                let pool = Arc::clone(&self.pool);
                tokio::spawn(async move {
                    pool.return_connection(client).await;
                });
            }
        }
    }

    struct ConnectionPoolInner {
        addr: String,
        config: PoolConfig,
        connections: Mutex<Vec<Client>>,
        semaphore: Semaphore,
    }

    impl ConnectionPoolInner {
        async fn return_connection(&self, client: Client) {
            let mut connections = self.connections.lock().await;
            if connections.len() < self.config.max_connections {
                connections.push(client);
            }
            self.semaphore.add_permits(1);
        }
    }

    /// Connection pool for MaharitDB
    pub struct ConnectionPool {
        inner: Arc<ConnectionPoolInner>,
    }

    impl ConnectionPool {
        /// Create a new connection pool
        pub async fn new(addr: &str, config: PoolConfig) -> Result<Self> {
            let inner = Arc::new(ConnectionPoolInner {
                addr: addr.to_string(),
                config: config.clone(),
                connections: Mutex::new(Vec::new()),
                semaphore: Semaphore::new(config.max_connections),
            });

            // Pre-create minimum connections
            let mut initial_connections = Vec::new();
            for _ in 0..config.min_connections {
                let client = Client::connect_with_config(addr, config.client_config.clone()).await?;
                initial_connections.push(client);
            }

            {
                let mut connections = inner.connections.lock().await;
                *connections = initial_connections;
            }

            Ok(Self { inner })
        }

        /// Get a connection from the pool
        pub async fn get(&self) -> Result<PooledConnection> {
            // Wait for an available slot
            let permit = self.inner.semaphore.acquire().await.map_err(|_| {
                ClientError::Protocol("Pool closed".to_string())
            })?;
            permit.forget(); // We'll add it back when connection is returned

            let mut connections = self.inner.connections.lock().await;

            let client = if let Some(client) = connections.pop() {
                client
            } else {
                // Create a new connection
                drop(connections); // Release lock before connecting
                Client::connect_with_config(
                    &self.inner.addr,
                    self.inner.config.client_config.clone(),
                )
                .await?
            };

            Ok(PooledConnection {
                client: Some(client),
                pool: Arc::clone(&self.inner),
            })
        }

        /// Get the number of idle connections in the pool
        pub async fn idle_count(&self) -> usize {
            self.inner.connections.lock().await.len()
        }

        /// Get the maximum pool size
        pub fn max_size(&self) -> usize {
            self.inner.config.max_connections
        }
    }
}

/// Synchronous client wrapper
pub mod sync {
    use super::*;

    /// Synchronous MaharitDB client
    pub struct SyncClient {
        runtime: tokio::runtime::Runtime,
        client: Client,
    }

    impl SyncClient {
        /// Connect to a MaharitDB server
        pub fn connect(addr: &str) -> Result<Self> {
            Self::connect_with_config(addr, ClientConfig::default())
        }

        /// Connect to a MaharitDB server with custom configuration
        pub fn connect_with_config(addr: &str, config: ClientConfig) -> Result<Self> {
            let runtime =
                tokio::runtime::Runtime::new().map_err(|e| ClientError::Connection(e.into()))?;

            let client = runtime.block_on(Client::connect_with_config(addr, config))?;

            Ok(Self { runtime, client })
        }

        /// Execute a query and return the result
        pub fn query(&mut self, query: &str) -> Result<QueryResult> {
            self.runtime.block_on(self.client.query(query))
        }

        /// Execute a query within a transaction
        pub fn query_in_tx(&mut self, query: &str, tx_id: Option<TxId>) -> Result<QueryResult> {
            self.runtime.block_on(self.client.query_in_tx(query, tx_id))
        }

        /// Execute a query without returning results
        pub fn execute(&mut self, query: &str) -> Result<()> {
            self.runtime.block_on(self.client.execute(query))
        }

        /// Execute a query within a transaction without returning results
        pub fn execute_in_tx(&mut self, query: &str, tx_id: TxId) -> Result<()> {
            self.runtime
                .block_on(self.client.execute_in_tx(query, tx_id))
        }

        /// Begin a new transaction
        pub fn begin(&mut self) -> Result<TxId> {
            self.runtime.block_on(self.client.begin())
        }

        /// Begin a read-only transaction
        pub fn begin_read_only(&mut self) -> Result<TxId> {
            self.runtime.block_on(self.client.begin_read_only())
        }

        /// Commit a transaction
        pub fn commit(&mut self, tx_id: TxId) -> Result<()> {
            self.runtime.block_on(self.client.commit(tx_id))
        }

        /// Rollback a transaction
        pub fn rollback(&mut self, tx_id: TxId) -> Result<()> {
            self.runtime.block_on(self.client.rollback(tx_id))
        }

        /// Ping the server
        pub fn ping(&mut self) -> Result<()> {
            self.runtime.block_on(self.client.ping())
        }

        /// Get server statistics
        pub fn stats(&mut self) -> Result<ServerStats> {
            self.runtime.block_on(self.client.stats())
        }

        /// Check if the connection is alive
        pub fn is_alive(&mut self) -> bool {
            self.runtime.block_on(self.client.is_alive())
        }

        /// Execute a query with streaming and collect all results
        ///
        /// This is a convenience method that streams results in chunks but
        /// collects them all into a single QueryResult.
        pub fn query_stream(&mut self, query: &str, chunk_size: usize) -> Result<QueryResult> {
            self.runtime.block_on(async {
                let stream = self.client.query_stream(query, chunk_size).await?;
                let rows = stream.collect_all().await?;
                Ok(QueryResult { rows })
            })
        }

        /// Execute a streaming query within a transaction and collect all results
        pub fn query_stream_in_tx(
            &mut self,
            query: &str,
            tx_id: Option<TxId>,
            chunk_size: usize,
        ) -> Result<QueryResult> {
            self.runtime.block_on(async {
                let stream = self.client.query_stream_in_tx(query, tx_id, chunk_size).await?;
                let rows = stream.collect_all().await?;
                Ok(QueryResult { rows })
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_result() {
        let mut row = HashMap::new();
        row.insert("name".to_string(), "Alice".to_string());
        row.insert("age".to_string(), "30".to_string());

        let result = QueryResult { rows: vec![row] };

        assert_eq!(result.row_count(), 1);
        assert!(!result.is_empty());
        assert_eq!(result.get_single("name"), Some(&"Alice".to_string()));
        assert_eq!(result.get_single("age"), Some(&"30".to_string()));
        assert_eq!(result.get_single("unknown"), None);
    }

    #[test]
    fn test_empty_result() {
        let result = QueryResult { rows: vec![] };

        assert_eq!(result.row_count(), 0);
        assert!(result.is_empty());
        assert_eq!(result.get_single("name"), None);
    }

    #[test]
    fn test_default_config() {
        let config = ClientConfig::default();
        assert_eq!(config.connect_timeout, Duration::from_secs(10));
        assert_eq!(config.read_timeout, Duration::from_secs(30));
        assert_eq!(config.write_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_request_serialization() {
        let request = Request::Query {
            query: "MATCH (n) RETURN n".to_string(),
            tx_id: None,
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"type\":\"query\""));
        assert!(json.contains("MATCH (n) RETURN n"));
        // tx_id should not be serialized when None
        assert!(!json.contains("txId"));
    }

    #[test]
    fn test_request_serialization_with_tx_id() {
        let request = Request::Query {
            query: "MATCH (n) RETURN n".to_string(),
            tx_id: Some(42),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"type\":\"query\""));
        assert!(json.contains("\"txId\":42"));
    }

    #[test]
    fn test_begin_transaction_request() {
        let request = Request::BeginTransaction { read_only: false };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"type\":\"begin\""));
        assert!(json.contains("\"readOnly\":false"));
    }

    #[test]
    fn test_commit_request() {
        let request = Request::Commit { tx_id: 123 };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"type\":\"commit\""));
        assert!(json.contains("\"txId\":123"));
    }

    #[test]
    fn test_rollback_request() {
        let request = Request::Rollback { tx_id: 456 };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"type\":\"rollback\""));
        assert!(json.contains("\"txId\":456"));
    }

    #[test]
    fn test_transaction_begun_response() {
        let json = r#"{"type":"transactionBegun","txId":42}"#;
        let response: Response = serde_json::from_str(json).unwrap();
        match response {
            Response::TransactionBegun { tx_id } => assert_eq!(tx_id, 42),
            _ => panic!("Expected TransactionBegun response"),
        }
    }

    #[test]
    fn test_committed_response() {
        let json = r#"{"type":"committed","txId":42}"#;
        let response: Response = serde_json::from_str(json).unwrap();
        match response {
            Response::Committed { tx_id } => assert_eq!(tx_id, 42),
            _ => panic!("Expected Committed response"),
        }
    }

    #[test]
    fn test_rolled_back_response() {
        let json = r#"{"type":"rolledBack","txId":42}"#;
        let response: Response = serde_json::from_str(json).unwrap();
        match response {
            Response::RolledBack { tx_id } => assert_eq!(tx_id, 42),
            _ => panic!("Expected RolledBack response"),
        }
    }

    #[test]
    fn test_response_deserialization() {
        let json = r#"{"type":"result","rows":[{"name":"Alice"}]}"#;
        let response: Response = serde_json::from_str(json).unwrap();
        match response {
            Response::Result { rows } => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].get("name"), Some(&"Alice".to_string()));
            }
            _ => panic!("Expected Result response"),
        }
    }

    #[test]
    fn test_pong_response() {
        let json = r#"{"type":"pong"}"#;
        let response: Response = serde_json::from_str(json).unwrap();
        assert!(matches!(response, Response::Pong));
    }

    #[test]
    fn test_error_response() {
        let json = r#"{"type":"error","message":"Something went wrong"}"#;
        let response: Response = serde_json::from_str(json).unwrap();
        match response {
            Response::Error { message } => {
                assert_eq!(message, "Something went wrong");
            }
            _ => panic!("Expected Error response"),
        }
    }

    #[test]
    fn test_config_with_auto_reconnect() {
        let config = ClientConfig {
            auto_reconnect: true,
            max_reconnect_attempts: 5,
            reconnect_delay: Duration::from_secs(1),
            ..Default::default()
        };

        assert!(config.auto_reconnect);
        assert_eq!(config.max_reconnect_attempts, 5);
        assert_eq!(config.reconnect_delay, Duration::from_secs(1));
    }

    #[test]
    fn test_pool_config_default() {
        let config = pool::PoolConfig::default();
        assert_eq!(config.min_connections, 1);
        assert_eq!(config.max_connections, 10);
        assert_eq!(config.idle_timeout, Duration::from_secs(300));
    }
}
