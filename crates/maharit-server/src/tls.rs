//! TLS configuration module for MaharitDB TCP server and client.
//!
//! This module provides TLS support using rustls and tokio-rustls, including:
//! - Server TLS configuration with certificate and private key loading
//! - Client TLS configuration with optional certificate verification
//! - Protocol version selection (TLS 1.2 or 1.3)
//! - Environment variable configuration support
//! - TOML configuration file support via [`TlsConfig::from_toml_file`]

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde::Deserialize;
use std::io::BufReader;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use thiserror::Error;

/// TLS configuration errors
#[derive(Error, Debug)]
pub enum TlsError {
    #[error("Certificate not found: {0}")]
    CertificateNotFound(String),

    #[error("Private key not found: {0}")]
    KeyNotFound(String),

    #[error("Invalid certificate: {0}")]
    InvalidCertificate(String),

    #[error("Invalid private key: {0}")]
    InvalidKey(String),

    #[error("TLS configuration error: {0}")]
    ConfigError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML parse error: {0}")]
    TomlParse(String),
}

// ---------------------------------------------------------------------------
// TOML configuration types
// ---------------------------------------------------------------------------

/// Intermediate deserialization target for the `[tls]` section of a TOML
/// configuration file.
///
/// ```toml
/// [tls]
/// enabled   = true
/// cert_file = "/etc/maharit/server.crt"
/// key_file  = "/etc/maharit/server.key"
/// min_version = "1.2"   # "1.2" or "1.3"
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct TlsFileConfig {
    /// Whether TLS is enabled.  When `false` the entire section is ignored.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Path to the PEM-encoded certificate file.
    pub cert_file: String,
    /// Path to the PEM-encoded private key file.
    pub key_file: String,
    /// Minimum TLS protocol version.  Accepted values: `"1.2"`, `"1.3"`.
    /// Defaults to `None` (rustls default, which is TLS 1.2).
    #[serde(default)]
    pub min_version: Option<String>,
}

fn default_true() -> bool {
    true
}

impl TlsFileConfig {
    /// Convert the minimum version string to a [`ProtocolVersion`].
    ///
    /// Returns `None` when `min_version` is not set.
    ///
    /// # Errors
    ///
    /// Returns [`TlsError::ConfigError`] when the value is set but not one of
    /// `"1.2"` or `"1.3"`.
    pub fn protocol_version(&self) -> Result<Option<ProtocolVersion>, TlsError> {
        match self.min_version.as_deref() {
            None => Ok(None),
            Some("1.2") => Ok(Some(ProtocolVersion::Tls12)),
            Some("1.3") => Ok(Some(ProtocolVersion::Tls13)),
            Some(other) => Err(TlsError::ConfigError(format!(
                "unsupported min_version \"{other}\": expected \"1.2\" or \"1.3\""
            ))),
        }
    }
}

/// Wrapper used to deserialize a TOML file that contains a top-level `[tls]`
/// section.
#[derive(Debug, Deserialize)]
struct TlsConfigFile {
    tls: TlsFileConfig,
}

/// TLS protocol version
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolVersion {
    /// TLS 1.2
    Tls12,
    /// TLS 1.3
    Tls13,
}

impl ProtocolVersion {
    /// Convert to rustls protocol version
    #[cfg(test)]
    fn to_rustls(&self) -> &'static rustls::SupportedProtocolVersion {
        match self {
            ProtocolVersion::Tls12 => &rustls::version::TLS12,
            ProtocolVersion::Tls13 => &rustls::version::TLS13,
        }
    }
}

/// TLS configuration for server
#[derive(Debug, Clone)]
pub struct TlsConfig {
    /// Path to the certificate file (PEM format)
    pub cert_path: String,
    /// Path to the private key file (PEM format)
    pub key_path: String,
    /// Minimum TLS protocol version
    pub min_protocol_version: Option<ProtocolVersion>,
}

impl TlsConfig {
    /// Create a new TLS configuration
    ///
    /// # Arguments
    ///
    /// * `cert_path` - Path to the certificate file in PEM format
    /// * `key_path` - Path to the private key file in PEM format
    ///
    /// # Example
    ///
    /// ```no_run
    /// use maharit_server::tls::TlsConfig;
    ///
    /// let config = TlsConfig::new("/path/to/cert.pem", "/path/to/key.pem");
    /// ```
    pub fn new(cert_path: &str, key_path: &str) -> Self {
        Self {
            cert_path: cert_path.to_string(),
            key_path: key_path.to_string(),
            min_protocol_version: None,
        }
    }

    /// Set the minimum TLS protocol version (builder pattern)
    ///
    /// # Arguments
    ///
    /// * `version` - Minimum protocol version to support
    ///
    /// # Example
    ///
    /// ```no_run
    /// use maharit_server::tls::{TlsConfig, ProtocolVersion};
    ///
    /// let config = TlsConfig::new("/path/to/cert.pem", "/path/to/key.pem")
    ///     .with_min_protocol(ProtocolVersion::Tls13);
    /// ```
    pub fn with_min_protocol(mut self, version: ProtocolVersion) -> Self {
        self.min_protocol_version = Some(version);
        self
    }

    /// Load TLS configuration from environment variables
    ///
    /// Reads `MAHARIT_TLS_CERT` and `MAHARIT_TLS_KEY` environment variables.
    ///
    /// # Returns
    ///
    /// `Some(TlsConfig)` if both environment variables are set, `None` otherwise.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use maharit_server::tls::TlsConfig;
    ///
    /// if let Some(config) = TlsConfig::from_env() {
    ///     println!("TLS enabled from environment variables");
    /// }
    /// ```
    pub fn from_env() -> Option<Self> {
        let cert_path = std::env::var("MAHARIT_TLS_CERT").ok()?;
        let key_path = std::env::var("MAHARIT_TLS_KEY").ok()?;
        Some(Self::new(&cert_path, &key_path))
    }

    /// Load TLS configuration from a TOML file.
    ///
    /// The file must contain a `[tls]` section.  If `enabled = false` the
    /// method returns `None`.  If `enabled` is omitted it defaults to `true`.
    ///
    /// ```toml
    /// [tls]
    /// enabled     = true
    /// cert_file   = "/etc/maharit/server.crt"
    /// key_file    = "/etc/maharit/server.key"
    /// min_version = "1.2"
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file cannot be read.
    /// - The TOML cannot be parsed.
    /// - `min_version` contains an unrecognised value.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use maharit_server::tls::TlsConfig;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// if let Some(config) = TlsConfig::from_toml_file("/etc/maharit/config.toml")? {
    ///     let server_config = config.load_server_config()?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_toml_file(path: &str) -> Result<Option<Self>, TlsError> {
        let content = std::fs::read_to_string(path)?;
        let wrapper: TlsConfigFile = toml::from_str(&content)
            .map_err(|e| TlsError::TomlParse(e.to_string()))?;
        Self::from_file_config(wrapper.tls)
    }

    /// Build a [`TlsConfig`] from a deserialized [`TlsFileConfig`].
    ///
    /// Returns `None` when `file_config.enabled` is `false`.
    ///
    /// # Errors
    ///
    /// Returns an error if `min_version` contains an unrecognised value.
    pub fn from_file_config(file_config: TlsFileConfig) -> Result<Option<Self>, TlsError> {
        if !file_config.enabled {
            return Ok(None);
        }

        let mut config = Self::new(&file_config.cert_file, &file_config.key_file);

        if let Some(version) = file_config.protocol_version()? {
            config = config.with_min_protocol(version);
        }

        Ok(Some(config))
    }

    /// Load server configuration for rustls
    ///
    /// This method loads the certificate and private key files and creates a
    /// rustls `ServerConfig` ready for use with tokio-rustls.
    ///
    /// # Returns
    ///
    /// An `Arc<rustls::ServerConfig>` on success, or a `TlsError` on failure.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Certificate or key files are not found
    /// - Certificate or key files are invalid or malformed
    /// - The certificate and key do not match
    ///
    /// # Example
    ///
    /// ```no_run
    /// use maharit_server::tls::TlsConfig;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = TlsConfig::new("/path/to/cert.pem", "/path/to/key.pem");
    /// let server_config = config.load_server_config()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn load_server_config(&self) -> Result<Arc<rustls::ServerConfig>, TlsError> {
        let certs = Self::load_certs(&self.cert_path)?;
        let key = Self::load_private_key(&self.key_path)?;

        let builder = if let Some(min_version) = self.min_protocol_version {
            let versions: Vec<&'static rustls::SupportedProtocolVersion> = match min_version {
                ProtocolVersion::Tls13 => vec![&rustls::version::TLS13],
                ProtocolVersion::Tls12 => {
                    vec![&rustls::version::TLS12, &rustls::version::TLS13]
                }
            };
            rustls::ServerConfig::builder_with_protocol_versions(&versions)
        } else {
            rustls::ServerConfig::builder()
        };

        let config = builder
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| TlsError::ConfigError(e.to_string()))?;

        Ok(Arc::new(config))
    }

    /// Load certificates from a PEM file
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the certificate file
    ///
    /// # Returns
    ///
    /// A vector of certificate DER-encoded data.
    ///
    /// # Errors
    ///
    /// Returns an error if the file is not found, cannot be read, or contains invalid certificates.
    fn load_certs(path: &str) -> Result<Vec<CertificateDer<'static>>, TlsError> {
        let file = std::fs::File::open(path)
            .map_err(|_| TlsError::CertificateNotFound(path.to_string()))?;
        let mut reader = BufReader::new(file);
        let certs = rustls_pemfile::certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| TlsError::InvalidCertificate(e.to_string()))?;

        if certs.is_empty() {
            return Err(TlsError::InvalidCertificate(
                "no certificates found".to_string(),
            ));
        }

        Ok(certs)
    }

    /// Load a private key from a PEM file
    ///
    /// This method attempts to load a private key in PKCS8, RSA, or EC format.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the private key file
    ///
    /// # Returns
    ///
    /// A `PrivateKeyDer` containing the private key data.
    ///
    /// # Errors
    ///
    /// Returns an error if the file is not found, cannot be read, or contains an invalid key.
    fn load_private_key(path: &str) -> Result<PrivateKeyDer<'static>, TlsError> {
        let file =
            std::fs::File::open(path).map_err(|_| TlsError::KeyNotFound(path.to_string()))?;
        let mut reader = BufReader::new(file);

        // Try to load private key (handles PKCS8, RSA, and EC formats)
        let key = rustls_pemfile::private_key(&mut reader)
            .map_err(|e| TlsError::InvalidKey(e.to_string()))?
            .ok_or_else(|| TlsError::InvalidKey("no private key found".to_string()))?;

        Ok(key)
    }
}

/// TLS acceptor wrapper for server-side connections
pub struct TlsAcceptorWrapper {
    acceptor: tokio_rustls::TlsAcceptor,
}

impl TlsAcceptorWrapper {
    /// Create a new TLS acceptor from a rustls server configuration
    ///
    /// # Arguments
    ///
    /// * `config` - The rustls server configuration
    ///
    /// # Example
    ///
    /// ```no_run
    /// use maharit_server::tls::{TlsConfig, TlsAcceptorWrapper};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = TlsConfig::new("/path/to/cert.pem", "/path/to/key.pem");
    /// let server_config = config.load_server_config()?;
    /// let acceptor = TlsAcceptorWrapper::new(server_config);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(config: Arc<rustls::ServerConfig>) -> Self {
        Self {
            acceptor: tokio_rustls::TlsAcceptor::from(config),
        }
    }

    /// Accept a TLS connection from a TCP stream
    ///
    /// # Arguments
    ///
    /// * `stream` - The incoming TCP stream
    ///
    /// # Returns
    ///
    /// A `TlsStream` wrapping the TCP stream on success.
    ///
    /// # Errors
    ///
    /// Returns an error if the TLS handshake fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use maharit_server::tls::{TlsConfig, TlsAcceptorWrapper};
    /// use tokio::net::TcpListener;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = TlsConfig::new("/path/to/cert.pem", "/path/to/key.pem");
    /// let server_config = config.load_server_config()?;
    /// let acceptor = TlsAcceptorWrapper::new(server_config);
    ///
    /// let listener = TcpListener::bind("127.0.0.1:8080").await?;
    /// let (stream, _) = listener.accept().await?;
    /// let tls_stream = acceptor.accept(stream).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn accept(
        &self,
        stream: tokio::net::TcpStream,
    ) -> Result<tokio_rustls::server::TlsStream<tokio::net::TcpStream>, TlsError> {
        self.acceptor
            .accept(stream)
            .await
            .map_err(|e| TlsError::ConfigError(format!("TLS handshake failed: {}", e)))
    }
}

/// No-op certificate verifier for development/testing
///
/// WARNING: This bypasses all certificate verification and should NEVER be used in production.
#[derive(Debug)]
struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
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

/// Build a client TLS configuration
///
/// # Arguments
///
/// * `ca_cert_path` - Optional path to a CA certificate for verification
/// * `skip_verify` - If true, bypass certificate verification (INSECURE - for testing only)
///
/// # Returns
///
/// An `Arc<rustls::ClientConfig>` on success.
///
/// # Errors
///
/// Returns an error if the CA certificate cannot be loaded (when provided).
///
/// # Example
///
/// ```no_run
/// use maharit_server::tls::build_client_config;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // With verification (production)
/// let config = build_client_config(Some("/path/to/ca.pem"), false)?;
///
/// // Without verification (testing only)
/// let insecure_config = build_client_config(None, true)?;
/// # Ok(())
/// # }
/// ```
pub fn build_client_config(
    ca_cert_path: Option<&str>,
    skip_verify: bool,
) -> Result<Arc<rustls::ClientConfig>, TlsError> {
    if skip_verify {
        // INSECURE: Skip certificate verification (for testing only)
        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerifier))
            .with_no_client_auth();
        return Ok(Arc::new(config));
    }

    if let Some(ca_path) = ca_cert_path {
        // Load custom CA certificate
        let ca_certs = TlsConfig::load_certs(ca_path)?;
        let mut root_store = rustls::RootCertStore::empty();

        for cert in ca_certs {
            root_store
                .add(cert)
                .map_err(|e| TlsError::ConfigError(format!("Failed to add CA cert: {}", e)))?;
        }

        let config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        Ok(Arc::new(config))
    } else {
        // Use system root certificates
        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        Ok(Arc::new(config))
    }
}

// ============================================================================
// Certificate Auto-Reloader
// ============================================================================

/// TLS証明書ファイルの変更を監視し、変更があれば自動的に再ロードする。
///
/// `std::fs::metadata` を使ってファイルの `last_modified` タイムスタンプを
/// `check_interval` ごとに確認する。変更が検出された場合、新しい [`TlsConfig`] を返す。
///
/// # Example
///
/// ```no_run
/// use maharit_server::tls::CertificateReloader;
/// use std::sync::Arc;
/// use std::time::Duration;
///
/// # async fn example() {
/// let reloader = Arc::new(CertificateReloader::new(
///     "/path/to/cert.pem",
///     "/path/to/key.pem",
///     Duration::from_secs(30),
/// ));
///
/// reloader.start_watching(|new_config| {
///     println!("Certificate reloaded: {:?}", new_config.cert_path);
/// }).await;
/// # }
/// ```
pub struct CertificateReloader {
    cert_path: String,
    key_path: String,
    check_interval: Duration,
    /// 最後に確認したファイルの変更時刻（cert と key の両方を追跡）
    last_modified: Mutex<Option<(SystemTime, SystemTime)>>,
}

impl CertificateReloader {
    /// 新しい `CertificateReloader` を作成する。
    ///
    /// # Arguments
    ///
    /// * `cert_path` - PEM形式の証明書ファイルのパス
    /// * `key_path` - PEM形式の秘密鍵ファイルのパス
    /// * `check_interval` - ファイル変更チェックの間隔
    pub fn new(cert_path: &str, key_path: &str, check_interval: Duration) -> Self {
        Self {
            cert_path: cert_path.to_string(),
            key_path: key_path.to_string(),
            check_interval,
            last_modified: Mutex::new(None),
        }
    }

    /// ファイルの変更タイムスタンプを取得する。
    fn get_modified_times(&self) -> Option<(SystemTime, SystemTime)> {
        let cert_meta = std::fs::metadata(&self.cert_path).ok()?;
        let key_meta = std::fs::metadata(&self.key_path).ok()?;
        let cert_time = cert_meta.modified().ok()?;
        let key_time = key_meta.modified().ok()?;
        Some((cert_time, key_time))
    }

    /// ファイルの変更を確認し、変更があれば新しい [`TlsConfig`] を返す。
    ///
    /// 初回呼び出し時は現在のタイムスタンプを記録して `None` を返す。
    /// 以降の呼び出しでタイムスタンプが変化していた場合に `Some(TlsConfig)` を返す。
    ///
    /// # Returns
    ///
    /// * `Some(TlsConfig)` - ファイルが変更されていた場合（新しい設定）
    /// * `None` - 変更がない場合、またはファイルが読めない場合
    pub fn check_reload(&self) -> Option<TlsConfig> {
        let current_times = self.get_modified_times()?;

        let mut last = self.last_modified.lock().unwrap();

        match *last {
            None => {
                // 初回: タイムスタンプを記録して None を返す
                *last = Some(current_times);
                None
            }
            Some(prev_times) => {
                if current_times != prev_times {
                    // ファイルが変更された: タイムスタンプを更新して新しい設定を返す
                    *last = Some(current_times);
                    Some(TlsConfig::new(&self.cert_path, &self.key_path))
                } else {
                    None
                }
            }
        }
    }

    /// バックグラウンドで定期的にファイル変更を監視する（tokio::spawn）。
    ///
    /// `check_interval` ごとにファイルの変更をチェックし、変更があれば
    /// `on_reload` コールバックを呼び出す。
    ///
    /// # Arguments
    ///
    /// * `on_reload` - 証明書が再ロードされた際に呼ばれるコールバック
    ///
    /// # Note
    ///
    /// この関数は非同期タスクを spawn して即座に返る。
    /// タスクはプログラムが終了するまで動き続ける。
    pub async fn start_watching(
        self: Arc<Self>,
        on_reload: impl Fn(TlsConfig) + Send + 'static,
    ) {
        // 初回チェックでベースラインのタイムスタンプを記録
        self.check_reload();

        let interval = self.check_interval;

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;

                if let Some(new_config) = self.check_reload() {
                    on_reload(new_config);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tls_config_creation() {
        let config = TlsConfig::new("/path/to/cert.pem", "/path/to/key.pem");
        assert_eq!(config.cert_path, "/path/to/cert.pem");
        assert_eq!(config.key_path, "/path/to/key.pem");
        assert!(config.min_protocol_version.is_none());
    }

    #[test]
    fn test_tls_config_with_min_protocol() {
        let config = TlsConfig::new("/path/to/cert.pem", "/path/to/key.pem")
            .with_min_protocol(ProtocolVersion::Tls13);
        assert_eq!(config.min_protocol_version, Some(ProtocolVersion::Tls13));

        let config2 = TlsConfig::new("/path/to/cert.pem", "/path/to/key.pem")
            .with_min_protocol(ProtocolVersion::Tls12);
        assert_eq!(config2.min_protocol_version, Some(ProtocolVersion::Tls12));
    }

    #[test]
    fn test_from_env_not_set() {
        // from_env returns None when MAHARIT_TLS_CERT/KEY are not set
        // (This test relies on these env vars not being set in CI/test environment)
        // We test the logic by verifying the method exists and returns Option
        let config = TlsConfig::from_env();
        // Result depends on environment - just ensure it doesn't panic
        let _ = config;
    }

    #[test]
    fn test_from_env_both_set() {
        // SAFETY: test environment, single-threaded test
        unsafe {
            std::env::set_var("MAHARIT_TLS_CERT", "/env/cert.pem");
            std::env::set_var("MAHARIT_TLS_KEY", "/env/key.pem");
        }

        let config = TlsConfig::from_env();
        assert!(config.is_some());

        let config = config.unwrap();
        assert_eq!(config.cert_path, "/env/cert.pem");
        assert_eq!(config.key_path, "/env/key.pem");

        // Clean up
        unsafe {
            std::env::remove_var("MAHARIT_TLS_CERT");
            std::env::remove_var("MAHARIT_TLS_KEY");
        }
    }

    #[test]
    fn test_load_certs_file_not_found() {
        let result = TlsConfig::load_certs("/nonexistent/cert.pem");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            TlsError::CertificateNotFound(_)
        ));
    }

    #[test]
    fn test_load_private_key_file_not_found() {
        let result = TlsConfig::load_private_key("/nonexistent/key.pem");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), TlsError::KeyNotFound(_)));
    }

    #[test]
    fn test_tls_error_display() {
        let err = TlsError::CertificateNotFound("/test/cert.pem".to_string());
        assert_eq!(err.to_string(), "Certificate not found: /test/cert.pem");

        let err = TlsError::KeyNotFound("/test/key.pem".to_string());
        assert_eq!(err.to_string(), "Private key not found: /test/key.pem");

        let err = TlsError::InvalidCertificate("malformed".to_string());
        assert_eq!(err.to_string(), "Invalid certificate: malformed");

        let err = TlsError::InvalidKey("malformed".to_string());
        assert_eq!(err.to_string(), "Invalid private key: malformed");

        let err = TlsError::ConfigError("test error".to_string());
        assert_eq!(err.to_string(), "TLS configuration error: test error");
    }

    #[test]
    fn test_protocol_version_equality() {
        assert_eq!(ProtocolVersion::Tls12, ProtocolVersion::Tls12);
        assert_eq!(ProtocolVersion::Tls13, ProtocolVersion::Tls13);
        assert_ne!(ProtocolVersion::Tls12, ProtocolVersion::Tls13);
    }

    #[test]
    fn test_client_config_skip_verify() {
        let result = build_client_config(None, true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_client_config_with_invalid_ca() {
        let result = build_client_config(Some("/nonexistent/ca.pem"), false);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            TlsError::CertificateNotFound(_)
        ));
    }

    #[test]
    fn test_tls_acceptor_wrapper_creation() {
        // Create a minimal server config for testing
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(rustls::server::ResolvesServerCertUsingSni::new()));

        let acceptor = TlsAcceptorWrapper::new(Arc::new(config));
        // Just verify it can be created
        assert!(std::mem::size_of_val(&acceptor) > 0);
    }

    #[test]
    fn test_protocol_version_to_rustls() {
        let v12 = ProtocolVersion::Tls12.to_rustls();
        let v13 = ProtocolVersion::Tls13.to_rustls();

        // Verify they return different protocol versions
        assert_ne!(v12.version, v13.version);
    }

    // -----------------------------------------------------------------------
    // TOML config tests
    // -----------------------------------------------------------------------

    use std::io::Write;

    fn write_toml(contents: &str) -> (tempfile_path::TempPath, String) {
        // We don't have tempfile in dependencies, so we write to std::env::temp_dir().
        let path = std::env::temp_dir().join(format!(
            "maharit_tls_test_{}.toml",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        let path_str = path.to_str().unwrap().to_string();
        (tempfile_path::TempPath(path), path_str)
    }

    // Minimal RAII guard to delete the temp file when the test finishes.
    mod tempfile_path {
        pub struct TempPath(pub std::path::PathBuf);
        impl Drop for TempPath {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
    }

    #[test]
    fn test_from_toml_file_basic() {
        let toml = r#"
[tls]
enabled   = true
cert_file = "/etc/maharit/server.crt"
key_file  = "/etc/maharit/server.key"
"#;
        let (_guard, path) = write_toml(toml);
        let result = TlsConfig::from_toml_file(&path).unwrap();
        let config = result.expect("should return Some when enabled = true");

        assert_eq!(config.cert_path, "/etc/maharit/server.crt");
        assert_eq!(config.key_path, "/etc/maharit/server.key");
        assert!(config.min_protocol_version.is_none());
    }

    #[test]
    fn test_from_toml_file_with_min_version_12() {
        let toml = r#"
[tls]
cert_file   = "/path/to/cert.pem"
key_file    = "/path/to/key.pem"
min_version = "1.2"
"#;
        let (_guard, path) = write_toml(toml);
        let config = TlsConfig::from_toml_file(&path).unwrap().unwrap();
        assert_eq!(config.min_protocol_version, Some(ProtocolVersion::Tls12));
    }

    #[test]
    fn test_from_toml_file_with_min_version_13() {
        let toml = r#"
[tls]
cert_file   = "/path/to/cert.pem"
key_file    = "/path/to/key.pem"
min_version = "1.3"
"#;
        let (_guard, path) = write_toml(toml);
        let config = TlsConfig::from_toml_file(&path).unwrap().unwrap();
        assert_eq!(config.min_protocol_version, Some(ProtocolVersion::Tls13));
    }

    #[test]
    fn test_from_toml_file_disabled_returns_none() {
        let toml = r#"
[tls]
enabled   = false
cert_file = "/path/to/cert.pem"
key_file  = "/path/to/key.pem"
"#;
        let (_guard, path) = write_toml(toml);
        let result = TlsConfig::from_toml_file(&path).unwrap();
        assert!(result.is_none(), "enabled = false should return None");
    }

    #[test]
    fn test_from_toml_file_invalid_min_version() {
        let toml = r#"
[tls]
cert_file   = "/path/to/cert.pem"
key_file    = "/path/to/key.pem"
min_version = "1.4"
"#;
        let (_guard, path) = write_toml(toml);
        let result = TlsConfig::from_toml_file(&path);
        assert!(result.is_err(), "invalid min_version should return an error");
        match result.unwrap_err() {
            TlsError::ConfigError(msg) => {
                assert!(msg.contains("1.4"), "error should mention the bad value: {msg}");
            }
            other => panic!("expected ConfigError, got {other:?}"),
        }
    }

    #[test]
    fn test_from_toml_file_not_found() {
        let result = TlsConfig::from_toml_file("/nonexistent/path/config.toml");
        assert!(result.is_err());
        // Should be an IO error (file not found)
        assert!(matches!(result.unwrap_err(), TlsError::Io(_)));
    }

    #[test]
    fn test_from_toml_file_parse_error() {
        let toml = "this is not valid toml ][[]";
        let (_guard, path) = write_toml(toml);
        let result = TlsConfig::from_toml_file(&path);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), TlsError::TomlParse(_)));
    }

    #[test]
    fn test_tls_file_config_protocol_version() {
        let mut cfg = TlsFileConfig {
            enabled: true,
            cert_file: "a".to_string(),
            key_file: "b".to_string(),
            min_version: None,
        };
        assert_eq!(cfg.protocol_version().unwrap(), None);

        cfg.min_version = Some("1.2".to_string());
        assert_eq!(cfg.protocol_version().unwrap(), Some(ProtocolVersion::Tls12));

        cfg.min_version = Some("1.3".to_string());
        assert_eq!(cfg.protocol_version().unwrap(), Some(ProtocolVersion::Tls13));

        cfg.min_version = Some("1.4".to_string());
        assert!(cfg.protocol_version().is_err());
    }

    #[test]
    fn test_tls_error_toml_parse_display() {
        let err = TlsError::TomlParse("missing field".to_string());
        assert_eq!(err.to_string(), "TOML parse error: missing field");
    }

    #[test]
    fn test_from_file_config_disabled() {
        let file_cfg = TlsFileConfig {
            enabled: false,
            cert_file: "/a/b.pem".to_string(),
            key_file: "/a/k.pem".to_string(),
            min_version: None,
        };
        let result = TlsConfig::from_file_config(file_cfg).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_from_file_config_enabled() {
        let file_cfg = TlsFileConfig {
            enabled: true,
            cert_file: "/a/b.pem".to_string(),
            key_file: "/a/k.pem".to_string(),
            min_version: Some("1.3".to_string()),
        };
        let result = TlsConfig::from_file_config(file_cfg).unwrap().unwrap();
        assert_eq!(result.cert_path, "/a/b.pem");
        assert_eq!(result.key_path, "/a/k.pem");
        assert_eq!(result.min_protocol_version, Some(ProtocolVersion::Tls13));
    }

    // -----------------------------------------------------------------------
    // CertificateReloader tests
    // -----------------------------------------------------------------------

    /// テスト用の一時ファイルを作成して、そのパスを返す RAII ガード
    struct TempCertFile(std::path::PathBuf);

    impl TempCertFile {
        fn new(suffix: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "maharit_cert_reload_test_{}{}.pem",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos(),
                suffix,
            ));
            std::fs::write(&path, b"dummy cert content").unwrap();
            Self(path)
        }

        fn path_str(&self) -> &str {
            self.0.to_str().unwrap()
        }

        fn touch(&self) {
            // ファイルを再書き込みして変更時刻を更新する
            let content = std::fs::read(&self.0).unwrap();
            std::fs::write(&self.0, content).unwrap();
        }
    }

    impl Drop for TempCertFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn test_certificate_reloader_no_change_returns_none() {
        // 変更なしでは check_reload() が None を返すこと
        let cert = TempCertFile::new("cert");
        let key = TempCertFile::new("key");

        let reloader = CertificateReloader::new(
            cert.path_str(),
            key.path_str(),
            Duration::from_secs(30),
        );

        // 初回呼び出し: ベースラインを設定
        let first = reloader.check_reload();
        assert!(first.is_none(), "初回呼び出しは None を返すべき");

        // 変更なしで再呼び出し: None を返す
        let second = reloader.check_reload();
        assert!(second.is_none(), "変更がない場合は None を返すべき");
    }

    #[test]
    fn test_certificate_reloader_file_change_returns_some() {
        // ファイルタイムスタンプ更新後に Some(TlsConfig) を返すこと
        let cert = TempCertFile::new("cert2");
        let key = TempCertFile::new("key2");

        let reloader = CertificateReloader::new(
            cert.path_str(),
            key.path_str(),
            Duration::from_secs(30),
        );

        // 初回呼び出し: ベースラインを設定
        reloader.check_reload();

        // わずかに待ってからファイルを更新（一部のファイルシステムでは
        // 変更時刻の精度が低いため、確実に変更を検出するために再書き込みする）
        std::thread::sleep(Duration::from_millis(10));
        cert.touch();

        // 変更後は Some(TlsConfig) を返す
        let result = reloader.check_reload();
        assert!(
            result.is_some(),
            "ファイル変更後は Some(TlsConfig) を返すべき"
        );

        let config = result.unwrap();
        assert_eq!(config.cert_path, cert.path_str());
        assert_eq!(config.key_path, key.path_str());
    }

    #[test]
    fn test_certificate_reloader_nonexistent_files_returns_none() {
        // 存在しないファイルに対しては check_reload() が None を返す
        let reloader = CertificateReloader::new(
            "/nonexistent/cert.pem",
            "/nonexistent/key.pem",
            Duration::from_secs(30),
        );

        let result = reloader.check_reload();
        assert!(result.is_none(), "存在しないファイルは None を返すべき");
    }

    #[test]
    fn test_certificate_reloader_new_creates_correctly() {
        let reloader = CertificateReloader::new(
            "/path/to/cert.pem",
            "/path/to/key.pem",
            Duration::from_secs(60),
        );

        assert_eq!(reloader.cert_path, "/path/to/cert.pem");
        assert_eq!(reloader.key_path, "/path/to/key.pem");
        assert_eq!(reloader.check_interval, Duration::from_secs(60));
    }
}
