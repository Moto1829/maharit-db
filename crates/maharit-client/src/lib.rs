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
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(30),
        }
    }
}

/// Transaction ID
pub type TxId = u64;

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
        })
    }

    /// Execute a query and return the result
    pub async fn query(&mut self, query: &str) -> Result<QueryResult> {
        self.query_in_tx(query, None).await
    }

    /// Execute a query within a transaction
    pub async fn query_in_tx(&mut self, query: &str, tx_id: Option<TxId>) -> Result<QueryResult> {
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
}
