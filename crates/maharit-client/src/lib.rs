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
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::{Buf, BytesMut};
use rustls::RootCertStore;
use rustls::pki_types::{CertificateDer, ServerName};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

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

    #[error("TLS error: {0}")]
    Tls(String),

    #[error("invalid certificate: {0}")]
    InvalidCertificate(String),

    #[error("invalid domain name: {0}")]
    InvalidDomain(String),
}

pub type Result<T> = std::result::Result<T, ClientError>;

/// TLS client configuration
#[derive(Debug, Clone)]
pub struct TlsClientConfig {
    /// Custom CA certificate path
    pub ca_cert_path: Option<String>,
    /// Skip certificate verification (for dev/testing - DO NOT use in production)
    pub skip_verify: bool,
    /// Server domain name for verification
    pub domain: String,
}

impl TlsClientConfig {
    /// Create a new TLS configuration with a domain name
    pub fn new(domain: impl Into<String>) -> Self {
        Self {
            ca_cert_path: None,
            skip_verify: false,
            domain: domain.into(),
        }
    }

    /// Set a custom CA certificate path
    pub fn with_ca_cert(mut self, path: impl Into<String>) -> Self {
        self.ca_cert_path = Some(path.into());
        self
    }

    /// Skip certificate verification (DANGEROUS - only for testing)
    pub fn with_skip_verify(mut self, skip: bool) -> Self {
        self.skip_verify = skip;
        self
    }
}

/// Dangerous certificate verifier that skips all verification
/// ONLY for development/testing - DO NOT use in production
#[derive(Debug)]
struct NoCertificateVerification;

impl rustls::client::danger::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PSS_SHA256,
        ]
    }
}

/// Connection stream that supports both plain TCP and TLS
#[allow(clippy::large_enum_variant)]
enum ConnectionStream {
    Plain(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl AsyncRead for ConnectionStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut *self {
            ConnectionStream::Plain(stream) => Pin::new(stream).poll_read(cx, buf),
            ConnectionStream::Tls(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ConnectionStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut *self {
            ConnectionStream::Plain(stream) => Pin::new(stream).poll_write(cx, buf),
            ConnectionStream::Tls(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            ConnectionStream::Plain(stream) => Pin::new(stream).poll_flush(cx),
            ConnectionStream::Tls(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            ConnectionStream::Plain(stream) => Pin::new(stream).poll_shutdown(cx),
            ConnectionStream::Tls(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

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
    #[serde(rename = "login")]
    Login { username: String, password: String },

    #[serde(rename = "query")]
    Query {
        query: String,
        #[serde(rename = "txId", skip_serializing_if = "Option::is_none")]
        tx_id: Option<TxId>,
        #[serde(rename = "sessionToken", skip_serializing_if = "Option::is_none")]
        session_token: Option<String>,
    },

    #[serde(rename = "streamQuery")]
    StreamQuery {
        query: String,
        #[serde(rename = "txId", skip_serializing_if = "Option::is_none")]
        tx_id: Option<TxId>,
        #[serde(rename = "chunkSize")]
        chunk_size: usize,
        #[serde(rename = "sessionToken", skip_serializing_if = "Option::is_none")]
        session_token: Option<String>,
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
        #[serde(rename = "sessionToken", skip_serializing_if = "Option::is_none")]
        session_token: Option<String>,
    },

    #[serde(rename = "commit")]
    Commit {
        #[serde(rename = "txId")]
        tx_id: TxId,
        #[serde(rename = "sessionToken", skip_serializing_if = "Option::is_none")]
        session_token: Option<String>,
    },

    #[serde(rename = "rollback")]
    Rollback {
        #[serde(rename = "txId")]
        tx_id: TxId,
        #[serde(rename = "sessionToken", skip_serializing_if = "Option::is_none")]
        session_token: Option<String>,
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
        #[allow(dead_code)]
        tx_id: TxId,
    },

    #[serde(rename = "rolledBack")]
    RolledBack {
        #[serde(rename = "txId")]
        #[allow(dead_code)]
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
        #[allow(dead_code)]
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

    #[serde(rename = "loggedIn")]
    LoggedIn {
        #[serde(rename = "sessionToken")]
        session_token: String,
        role: String,
        #[serde(rename = "expiresAt")]
        expires_at: u64,
    },

    #[serde(rename = "authError")]
    AuthError { message: String },
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

/// Login response information returned by `Client::login()`.
#[derive(Debug, Clone)]
pub struct LoginInfo {
    /// Session token to be carried in subsequent requests
    pub session_token: String,
    /// User role: "admin" / "read_write" / "read_only"
    pub role: String,
    /// Unix epoch seconds when this session expires
    pub expires_at: u64,
}

/// MaharitDB client
pub struct Client {
    stream: ConnectionStream,
    config: ClientConfig,
    buffer: BytesMut,
    addr: String,
    /// 認証成功後に保持するセッショントークン。`None` のときはトークン無しで送信。
    session_token: Option<String>,
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
            stream: ConnectionStream::Plain(stream),
            config,
            buffer: BytesMut::with_capacity(4096),
            addr: addr.to_string(),
            session_token: None,
        })
    }

    /// Connect to a MaharitDB server with TLS
    pub async fn connect_tls(addr: &str, tls_config: TlsClientConfig) -> Result<Self> {
        Self::connect_tls_with_config(addr, tls_config, ClientConfig::default()).await
    }

    /// Connect to a MaharitDB server with TLS and custom client configuration
    pub async fn connect_tls_with_config(
        addr: &str,
        tls_config: TlsClientConfig,
        client_config: ClientConfig,
    ) -> Result<Self> {
        // Create root certificate store
        let mut root_cert_store = RootCertStore::empty();

        if let Some(ca_cert_path) = &tls_config.ca_cert_path {
            // Load custom CA certificate
            let ca_cert_file = std::fs::File::open(ca_cert_path).map_err(|e| {
                ClientError::InvalidCertificate(format!("Failed to open CA cert file: {}", e))
            })?;
            let mut reader = std::io::BufReader::new(ca_cert_file);

            let certs = rustls_pemfile::certs(&mut reader)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| {
                    ClientError::InvalidCertificate(format!("Failed to parse CA cert: {}", e))
                })?;

            for cert in certs {
                root_cert_store.add(cert).map_err(|e| {
                    ClientError::InvalidCertificate(format!("Failed to add CA cert: {}", e))
                })?;
            }
        } else {
            // Use system certificates
            // Note: We don't use webpki-roots as it's not in dependencies
            // In production, you should either provide a CA cert or add webpki-roots
        }

        // Build TLS config
        let mut rustls_config = if tls_config.skip_verify {
            // Create a dangerous config that skips verification
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
                .with_no_client_auth()
        } else {
            rustls::ClientConfig::builder()
                .with_root_certificates(root_cert_store)
                .with_no_client_auth()
        };

        // Enable ALPN if needed
        rustls_config.alpn_protocols = vec![b"http/1.1".to_vec()];

        let connector = TlsConnector::from(Arc::new(rustls_config));

        // Connect TCP stream
        let tcp_stream = timeout(client_config.connect_timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| ClientError::Timeout)?
            .map_err(ClientError::Connection)?;

        // Parse domain name
        let server_name = ServerName::try_from(tls_config.domain.as_str())
            .map_err(|_| ClientError::InvalidDomain(tls_config.domain.clone()))?
            .to_owned();

        // Perform TLS handshake
        let tls_stream = connector
            .connect(server_name, tcp_stream)
            .await
            .map_err(|e| ClientError::Tls(format!("TLS handshake failed: {}", e)))?;

        Ok(Self {
            stream: ConnectionStream::Tls(tls_stream),
            config: client_config,
            buffer: BytesMut::with_capacity(4096),
            addr: addr.to_string(),
            session_token: None,
        })
    }

    /// Attempt to reconnect to the server
    /// Note: This only works for plain TCP connections. TLS reconnection is not supported.
    pub async fn reconnect(&mut self) -> Result<()> {
        match &self.stream {
            ConnectionStream::Plain(_) => {
                let stream = timeout(self.config.connect_timeout, TcpStream::connect(&self.addr))
                    .await
                    .map_err(|_| ClientError::Timeout)?
                    .map_err(ClientError::Connection)?;

                self.stream = ConnectionStream::Plain(stream);
                self.buffer.clear();
                Ok(())
            }
            ConnectionStream::Tls(_) => Err(ClientError::Protocol(
                "TLS reconnection not supported. Please create a new client.".to_string(),
            )),
        }
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
        matches!(
            err,
            ClientError::ConnectionClosed | ClientError::Connection(_)
        )
    }

    /// サーバーに `Login` リクエストを送り、認証成功時はセッショントークンを
    /// クライアントに保存して以降のリクエストに自動付与する。
    ///
    /// 認証エラーの場合は `ClientError::Server` で失敗する。
    pub async fn login(&mut self, username: &str, password: &str) -> Result<LoginInfo> {
        let request = Request::Login {
            username: username.to_string(),
            password: password.to_string(),
        };
        self.send_request(&request).await?;
        match self.receive_response().await? {
            Response::LoggedIn {
                session_token,
                role,
                expires_at,
            } => {
                self.session_token = Some(session_token.clone());
                Ok(LoginInfo {
                    session_token,
                    role,
                    expires_at,
                })
            }
            Response::AuthError { message } => Err(ClientError::Server(message)),
            Response::Error { message } => Err(ClientError::Server(message)),
            _ => Err(ClientError::Protocol(
                "unexpected response type for login".to_string(),
            )),
        }
    }

    /// 現在のセッショントークンを破棄する。サーバーに通知はしない
    /// （セッション失効はサーバー側のタイムアウトで自然に行われる）。
    pub fn logout(&mut self) {
        self.session_token = None;
    }

    /// クライアントに保持されているセッショントークンを返す（テスト用途）。
    pub fn session_token(&self) -> Option<&str> {
        self.session_token.as_deref()
    }

    /// Execute a query and return the result
    pub async fn query(&mut self, query: &str) -> Result<QueryResult> {
        self.query_in_tx(query, None).await
    }

    /// Execute a query within a transaction
    pub async fn query_in_tx(&mut self, query: &str, tx_id: Option<TxId>) -> Result<QueryResult> {
        let result = self.query_internal(query, tx_id).await;

        // Only auto-reconnect if not in a transaction
        if let Err(ref e) = result
            && self.config.auto_reconnect
            && tx_id.is_none()
            && Self::should_reconnect(e)
            && self.try_reconnect().await
        {
            return self.query_internal(query, tx_id).await;
        }

        result
    }

    async fn query_internal(&mut self, query: &str, tx_id: Option<TxId>) -> Result<QueryResult> {
        let request = Request::Query {
            query: query.to_string(),
            tx_id, session_token: self.session_token.clone(),
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
    pub async fn query_stream(
        &mut self,
        query: &str,
        chunk_size: usize,
    ) -> Result<StreamingResult<'_>> {
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
            chunk_size: if chunk_size == 0 {
                DEFAULT_CHUNK_SIZE
            } else {
                chunk_size
            }, session_token: self.session_token.clone(),
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

    /// Execute multiple queries sequentially on the same connection.
    ///
    /// Returns a `Vec<QueryResult>` in the same order as the input queries.
    /// Stops and returns an error if any query fails.
    ///
    /// # Example
    /// ```ignore
    /// let results = client.execute_many(&[
    ///     "CREATE (n:Person {name: 'Alice'})",
    ///     "CREATE (n:Person {name: 'Bob'})",
    ///     "MATCH (n:Person) RETURN n.name",
    /// ]).await?;
    /// ```
    pub async fn execute_many(&mut self, queries: &[&str]) -> Result<Vec<QueryResult>> {
        let mut results = Vec::with_capacity(queries.len());
        for q in queries {
            results.push(self.query(q).await?);
        }
        Ok(results)
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
        let request = Request::BeginTransaction { read_only, session_token: self.session_token.clone() };
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
        let request = Request::Commit { tx_id, session_token: self.session_token.clone() };
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
        let request = Request::Rollback { tx_id, session_token: self.session_token.clone() };
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

        if let Err(ref e) = result
            && self.config.auto_reconnect
            && Self::should_reconnect(e)
            && self.try_reconnect().await
        {
            return self.ping_internal().await;
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
                let client =
                    Client::connect_with_config(addr, config.client_config.clone()).await?;
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
            let permit = self
                .inner
                .semaphore
                .acquire()
                .await
                .map_err(|_| ClientError::Protocol("Pool closed".to_string()))?;
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
                tokio::runtime::Runtime::new().map_err(ClientError::Connection)?;

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
                let stream = self
                    .client
                    .query_stream_in_tx(query, tx_id, chunk_size)
                    .await?;
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
            tx_id: None, session_token: None,
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
            tx_id: Some(42), session_token: None,
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"type\":\"query\""));
        assert!(json.contains("\"txId\":42"));
    }

    #[test]
    fn test_begin_transaction_request() {
        let request = Request::BeginTransaction { read_only: false, session_token: None };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"type\":\"begin\""));
        assert!(json.contains("\"readOnly\":false"));
    }

    #[test]
    fn test_commit_request() {
        let request = Request::Commit { tx_id: 123, session_token: None };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"type\":\"commit\""));
        assert!(json.contains("\"txId\":123"));
    }

    #[test]
    fn test_rollback_request() {
        let request = Request::Rollback { tx_id: 456, session_token: None };
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

    #[test]
    fn test_tls_client_config_new() {
        let config = TlsClientConfig::new("example.com");
        assert_eq!(config.domain, "example.com");
        assert_eq!(config.ca_cert_path, None);
        assert!(!config.skip_verify);
    }

    #[test]
    fn test_tls_client_config_with_ca_cert() {
        let config = TlsClientConfig::new("example.com").with_ca_cert("/path/to/ca.crt");
        assert_eq!(config.ca_cert_path, Some("/path/to/ca.crt".to_string()));
        assert_eq!(config.domain, "example.com");
    }

    #[test]
    fn test_tls_client_config_with_skip_verify() {
        let config = TlsClientConfig::new("example.com").with_skip_verify(true);
        assert!(config.skip_verify);
        assert_eq!(config.domain, "example.com");
    }

    #[test]
    fn test_tls_client_config_chaining() {
        let config = TlsClientConfig::new("example.com")
            .with_ca_cert("/path/to/ca.crt")
            .with_skip_verify(true);
        assert_eq!(config.ca_cert_path, Some("/path/to/ca.crt".to_string()));
        assert!(config.skip_verify);
        assert_eq!(config.domain, "example.com");
    }

    #[tokio::test]
    async fn test_tls_connect_invalid_ca_cert_path() {
        let tls_config = TlsClientConfig::new("localhost").with_ca_cert("/nonexistent/path/ca.crt");

        let result = Client::connect_tls("localhost:7687", tls_config).await;
        assert!(result.is_err());

        if let Err(ClientError::InvalidCertificate(msg)) = result {
            assert!(msg.contains("Failed to open CA cert file"));
        } else {
            panic!("Expected InvalidCertificate error");
        }
    }

    #[tokio::test]
    async fn test_tls_connect_invalid_domain() {
        let tls_config = TlsClientConfig {
            ca_cert_path: None,
            skip_verify: false,
            domain: "".to_string(), // Invalid domain
        };

        let result = Client::connect_tls("localhost:7687", tls_config).await;
        // This might fail with either InvalidDomain or Connection error
        // depending on if domain validation happens before connection
        assert!(result.is_err());
    }

    #[test]
    fn test_connection_stream_enum_exists() {
        // This is a compile-time test to ensure ConnectionStream enum exists
        // We can't easily instantiate it in a synchronous test, but we can
        // verify the type exists by checking size
        assert!(std::mem::size_of::<ConnectionStream>() > 0);
    }

    #[test]
    fn test_stream_query_request_serialization() {
        let request = Request::StreamQuery {
            query: "MATCH (n) RETURN n".to_string(),
            tx_id: None,
            chunk_size: 100, session_token: None,
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"type\":\"streamQuery\""));
        assert!(json.contains("\"chunkSize\":100"));
        assert!(json.contains("MATCH (n) RETURN n"));
    }

    #[test]
    fn test_stream_start_response() {
        let json = r#"{"type":"streamStart","streamId":1,"totalRows":100}"#;
        let response: Response = serde_json::from_str(json).unwrap();
        match response {
            Response::StreamStart {
                stream_id,
                total_rows,
            } => {
                assert_eq!(stream_id, 1);
                assert_eq!(total_rows, Some(100));
            }
            _ => panic!("Expected StreamStart response"),
        }
    }

    #[test]
    fn test_stream_chunk_response() {
        let json =
            r#"{"type":"streamChunk","streamId":1,"chunkIndex":0,"rows":[{"name":"Alice"}]}"#;
        let response: Response = serde_json::from_str(json).unwrap();
        match response {
            Response::StreamChunk {
                stream_id,
                chunk_index,
                rows,
            } => {
                assert_eq!(stream_id, 1);
                assert_eq!(chunk_index, 0);
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].get("name"), Some(&"Alice".to_string()));
            }
            _ => panic!("Expected StreamChunk response"),
        }
    }

    #[test]
    fn test_stream_end_response() {
        let json = r#"{"type":"streamEnd","streamId":1,"totalRows":100}"#;
        let response: Response = serde_json::from_str(json).unwrap();
        match response {
            Response::StreamEnd {
                stream_id,
                total_rows,
            } => {
                assert_eq!(stream_id, 1);
                assert_eq!(total_rows, 100);
            }
            _ => panic!("Expected StreamEnd response"),
        }
    }

    #[test]
    fn test_login_request_serialization() {
        let request = Request::Login {
            username: "alice".to_string(),
            password: "secret".to_string(),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"type\":\"login\""));
        assert!(json.contains("\"username\":\"alice\""));
        assert!(json.contains("\"password\":\"secret\""));
    }

    #[test]
    fn test_query_with_session_token_serialization() {
        let request = Request::Query {
            query: "MATCH (n) RETURN n".to_string(),
            tx_id: None,
            session_token: Some("abc-xyz".to_string()),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"sessionToken\":\"abc-xyz\""));
    }

    #[test]
    fn test_query_without_session_token_omits_field() {
        let request = Request::Query {
            query: "MATCH (n) RETURN n".to_string(),
            tx_id: None,
            session_token: None,
        };
        let json = serde_json::to_string(&request).unwrap();
        // skip_serializing_if により sessionToken はシリアライズされない
        assert!(!json.contains("sessionToken"));
    }

    #[test]
    fn test_logged_in_response_deserialization() {
        let json = r#"{"type":"loggedIn","sessionToken":"tok-1","role":"admin","expiresAt":1700000000}"#;
        let resp: Response = serde_json::from_str(json).unwrap();
        match resp {
            Response::LoggedIn {
                session_token,
                role,
                expires_at,
            } => {
                assert_eq!(session_token, "tok-1");
                assert_eq!(role, "admin");
                assert_eq!(expires_at, 1_700_000_000);
            }
            _ => panic!("Expected LoggedIn response"),
        }
    }

    #[test]
    fn test_auth_error_response_deserialization() {
        let json = r#"{"type":"authError","message":"missing sessionToken"}"#;
        let resp: Response = serde_json::from_str(json).unwrap();
        match resp {
            Response::AuthError { message } => {
                assert_eq!(message, "missing sessionToken");
            }
            _ => panic!("Expected AuthError response"),
        }
    }
}
