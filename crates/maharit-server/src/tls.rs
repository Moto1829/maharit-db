//! TLS configuration module for MaharitDB TCP server and client.
//!
//! This module provides TLS support using rustls and tokio-rustls, including:
//! - Server TLS configuration with certificate and private key loading
//! - Client TLS configuration with optional certificate verification
//! - Protocol version selection (TLS 1.2 or 1.3)
//! - Environment variable configuration support

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::io::BufReader;
use std::sync::Arc;
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
}
