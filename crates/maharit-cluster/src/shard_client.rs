//! Shard client: sends Cypher queries to a remote shard node over TCP.
//!
//! Uses the same JSON + 4-byte length-prefix framing as `maharit-server`'s
//! TcpServer so that any shard node can be queried transparently, optionally
//! over TLS and logged in to shards that require authentication.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
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
        #[serde(rename = "sessionToken", skip_serializing_if = "Option::is_none")]
        session_token: Option<String>,
    },
    #[serde(rename = "login")]
    Login { username: String, password: String },
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum ShardResponse {
    #[serde(rename = "result")]
    Result {
        rows: Vec<HashMap<String, serde_json::Value>>,
    },
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(rename = "authError")]
    AuthError { message: String },
    #[serde(rename = "loggedIn")]
    LoggedIn {
        #[serde(rename = "sessionToken")]
        session_token: String,
    },
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
    #[error("TLS error: {0}")]
    Tls(String),
    #[error("Shard rejected the credentials or session: {0}")]
    Auth(String),
}

// ─── Connection options ──────────────────────────────────────────────────────

/// How a [`ShardClient`] connects: TLS and credentials.
#[derive(Clone, Default)]
pub struct ShardConnectOptions {
    /// When set, connect over TLS and verify the shard's certificate against
    /// this configuration, for the host part of the shard address.
    pub tls: Option<Arc<rustls::ClientConfig>>,
    /// When set, log in with `(username, password)` after connecting and send
    /// the session token with every query.
    pub credentials: Option<(String, String)>,
}

impl std::fmt::Debug for ShardConnectOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShardConnectOptions")
            .field("tls", &self.tls.is_some())
            .field("credentials", &self.credentials.as_ref().map(|(u, _)| u))
            .finish()
    }
}

/// A byte stream to a shard: plain TCP or TLS.
trait ShardStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> ShardStream for T {}

// ─── ShardClient ─────────────────────────────────────────────────────────────

/// TCP client that targets a single shard node.
///
/// The connection is established lazily on the first call to [`execute`] and
/// reused across subsequent calls.  On any I/O error the connection is dropped
/// and re-established transparently on the next retry.
pub struct ShardClient {
    addr: String,
    stream: Option<Box<dyn ShardStream>>,
    connect_timeout: Duration,
    read_timeout: Duration,
    options: ShardConnectOptions,
    /// Session token of the current connection (when logging in).
    session_token: Option<String>,
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
            options: ShardConnectOptions::default(),
            session_token: None,
        }
    }

    /// Use TLS and/or credentials for this shard.
    pub fn with_options(mut self, options: ShardConnectOptions) -> Self {
        self.options = options;
        self
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
        let tcp = timeout(self.connect_timeout, TcpStream::connect(&self.addr))
            .await
            .map_err(|_| ShardClientError::Timeout)??;
        let stream: Box<dyn ShardStream> = match &self.options.tls {
            None => Box::new(tcp),
            Some(config) => {
                let host = self
                    .addr
                    .rsplit_once(':')
                    .map_or(self.addr.as_str(), |(h, _)| h)
                    .trim_start_matches('[')
                    .trim_end_matches(']');
                let name = rustls::pki_types::ServerName::try_from(host.to_string())
                    .map_err(|e| ShardClientError::Tls(format!("invalid host {host}: {e}")))?;
                let connector = tokio_rustls::TlsConnector::from(Arc::clone(config));
                let tls = timeout(self.connect_timeout, connector.connect(name, tcp))
                    .await
                    .map_err(|_| ShardClientError::Timeout)?
                    .map_err(|e| ShardClientError::Tls(format!("handshake failed: {e}")))?;
                Box::new(tls)
            }
        };
        self.stream = Some(stream);
        self.session_token = None;
        if let Some((username, password)) = self.options.credentials.clone() {
            let resp = self
                .send_recv_raw(&ShardRequest::Login { username, password })
                .await?;
            match resp {
                ShardResponse::LoggedIn { session_token } => {
                    self.session_token = Some(session_token);
                }
                ShardResponse::AuthError { message } | ShardResponse::Error { message } => {
                    self.stream = None;
                    return Err(ShardClientError::Auth(message));
                }
                _ => {
                    self.stream = None;
                    return Err(ShardClientError::Auth("unexpected login response".into()));
                }
            }
        }
        Ok(())
    }

    /// Exchange one request/response on the current connection.
    async fn send_recv_raw(
        &mut self,
        req: &ShardRequest,
    ) -> Result<ShardResponse, ShardClientError> {
        let payload = serde_json::to_vec(req)?;
        let len_bytes = (payload.len() as u32).to_be_bytes();

        let Some(stream) = self.stream.as_mut() else {
            return Err(ShardClientError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "not connected",
            )));
        };
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

    /// Send `query` with the current session token.
    async fn query_once(&mut self, query: &str) -> Result<ShardResponse, ShardClientError> {
        self.ensure_connected().await?;
        let req = ShardRequest::Query {
            query: query.to_string(),
            tx_id: None,
            session_token: self.session_token.clone(),
        };
        self.send_recv_raw(&req).await
    }

    // ── Public API ───────────────────────────────────────────────────────────

    /// Execute a Cypher query on the target shard and return the result rows.
    ///
    /// On a transient connection failure the method reconnects and retries
    /// once before returning an error.
    pub async fn execute(&mut self, query: &str) -> Result<Vec<Row>, ShardClientError> {
        let mut resp = match self.query_once(query).await {
            Err(_) => {
                self.stream = None;
                self.query_once(query).await?
            }
            Ok(r) => r,
        };
        // The session may have expired: log in again on a new connection.
        if matches!(resp, ShardResponse::AuthError { .. }) && self.options.credentials.is_some() {
            self.stream = None;
            resp = self.query_once(query).await?;
        }

        match resp {
            ShardResponse::Result { rows } => Ok(rows
                .into_iter()
                .map(|cols| {
                    let mut row = Row::new();
                    for (k, v) in cols {
                        row.insert(k, RowValue::from_json(v));
                    }
                    row
                })
                .collect()),
            ShardResponse::Error { message } => Err(ShardClientError::Server(message)),
            ShardResponse::AuthError { message } => Err(ShardClientError::Auth(message)),
            ShardResponse::LoggedIn { .. } | ShardResponse::Other => Err(ShardClientError::Server(
                "unexpected response to a query".to_string(),
            )),
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
