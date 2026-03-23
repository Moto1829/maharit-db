//! Shard client: sends Cypher queries to a remote shard node over TCP.
//!
//! Uses the same JSON + 4-byte length-prefix framing as `maharit-server`'s
//! TcpServer so that any shard node can be queried transparently.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::coordinator::{Row, RowValue};

// ─── Wire types ──────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(tag = "type")]
enum ShardRequest {
    #[serde(rename = "query")]
    Query {
        query: String,
        #[serde(rename = "txId")]
        tx_id: Option<u64>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum ShardResponse {
    #[serde(rename = "result")]
    Result { rows: Vec<HashMap<String, String>> },
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(other)]
    Other,
}

// ─── Error ───────────────────────────────────────────────────────────────────

/// Error returned by [`ShardClient`] operations.
#[derive(Debug, thiserror::Error)]
pub enum ShardClientError {
    #[error("I/O error communicating with shard: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON (de)serialisation error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Shard returned an error: {0}")]
    Server(String),
    #[error("Timed out connecting to or reading from shard")]
    Timeout,
}

// ─── ShardClient ─────────────────────────────────────────────────────────────

/// TCP client that targets a single shard node.
///
/// The connection is established lazily on the first call to [`execute`] and
/// reused across subsequent calls.  On any I/O error the connection is dropped
/// and re-established transparently on the next retry.
pub struct ShardClient {
    addr: String,
    stream: Option<TcpStream>,
    connect_timeout: Duration,
    read_timeout: Duration,
}

impl ShardClient {
    /// Create a new `ShardClient` targeting `addr` (e.g. `"127.0.0.1:7687"`).
    ///
    /// The TCP connection is not made until [`execute`] is called.
    pub fn new(addr: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            stream: None,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(30),
        }
    }

    /// Return the target address of this client.
    pub fn addr(&self) -> &str {
        &self.addr
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    async fn ensure_connected(&mut self) -> Result<(), ShardClientError> {
        if self.stream.is_some() {
            return Ok(());
        }
        let stream = timeout(self.connect_timeout, TcpStream::connect(&self.addr))
            .await
            .map_err(|_| ShardClientError::Timeout)??;
        self.stream = Some(stream);
        Ok(())
    }

    async fn send_recv(&mut self, req: &ShardRequest) -> Result<ShardResponse, ShardClientError> {
        self.ensure_connected().await?;

        let payload = serde_json::to_vec(req)?;
        let len_bytes = (payload.len() as u32).to_be_bytes();

        let stream = self.stream.as_mut().unwrap();
        if let Err(e) = stream.write_all(&len_bytes).await {
            self.stream = None;
            return Err(ShardClientError::Io(e));
        }
        if let Err(e) = stream.write_all(&payload).await {
            self.stream = None;
            return Err(ShardClientError::Io(e));
        }

        // Read response length prefix.
        let mut len_buf = [0u8; 4];
        let read_len = timeout(self.read_timeout, stream.read_exact(&mut len_buf)).await;
        match read_len {
            Err(_) => {
                self.stream = None;
                return Err(ShardClientError::Timeout);
            }
            Ok(Err(e)) => {
                self.stream = None;
                return Err(ShardClientError::Io(e));
            }
            Ok(Ok(_)) => {}
        }

        let msg_len = u32::from_be_bytes(len_buf) as usize;
        let mut msg_buf = vec![0u8; msg_len];
        let read_body = timeout(self.read_timeout, stream.read_exact(&mut msg_buf)).await;
        match read_body {
            Err(_) => {
                self.stream = None;
                return Err(ShardClientError::Timeout);
            }
            Ok(Err(e)) => {
                self.stream = None;
                return Err(ShardClientError::Io(e));
            }
            Ok(Ok(_)) => {}
        }

        Ok(serde_json::from_slice(&msg_buf)?)
    }

    // ── Public API ───────────────────────────────────────────────────────────

    /// Execute a Cypher query on the target shard and return the result rows.
    ///
    /// On a transient connection failure the method reconnects and retries
    /// once before returning an error.
    pub async fn execute(&mut self, query: &str) -> Result<Vec<Row>, ShardClientError> {
        let req = ShardRequest::Query {
            query: query.to_string(),
            tx_id: None,
        };

        let resp = match self.send_recv(&req).await {
            Err(_) => {
                self.stream = None;
                self.send_recv(&req).await?
            }
            Ok(r) => r,
        };

        match resp {
            ShardResponse::Result { rows } => Ok(rows
                .into_iter()
                .map(|cols| {
                    let mut row = Row::new();
                    for (k, v) in cols {
                        row.insert(k, RowValue::Text(v));
                    }
                    row
                })
                .collect()),
            ShardResponse::Error { message } => Err(ShardClientError::Server(message)),
            ShardResponse::Other => Ok(vec![]),
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shard_client_new() {
        let c = ShardClient::new("127.0.0.1:7687");
        assert_eq!(c.addr(), "127.0.0.1:7687");
        assert!(c.stream.is_none());
    }

    #[test]
    fn test_shard_client_addr() {
        let c = ShardClient::new("localhost:9000");
        assert_eq!(c.addr(), "localhost:9000");
    }

    #[tokio::test]
    async fn test_execute_connection_refused() {
        // Port that should not be listening.
        let mut c = ShardClient::new("127.0.0.1:1");
        let result = c.execute("MATCH (n) RETURN n").await;
        assert!(result.is_err());
    }
}
