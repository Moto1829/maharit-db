# TLS Connection Support for maharit-client

## Task
Add TLS connection support to the maharit-client crate.

## Implementation Details

### Added Components

1. **TlsClientConfig struct** - Configuration for TLS connections
   - `ca_cert_path: Option<String>` - Custom CA certificate path
   - `skip_verify: bool` - Skip certificate verification (for dev/testing)
   - `domain: String` - Server domain name for verification
   - Builder methods: `new()`, `with_ca_cert()`, `with_skip_verify()`

2. **ConnectionStream enum** - Wrapper supporting both plain and TLS streams
   - `Plain(TcpStream)` - Plain TCP connection
   - `Tls(TlsStream<TcpStream>)` - TLS-encrypted connection
   - Implements `AsyncRead` and `AsyncWrite` traits

3. **NoCertificateVerification struct** - Dangerous verifier for skip_verify mode
   - Only for development/testing
   - Implements `rustls::client::danger::ServerCertVerifier`

4. **Client methods**
   - `connect_tls(addr, tls_config)` - Connect with TLS using default client config
   - `connect_tls_with_config(addr, tls_config, client_config)` - Connect with TLS and custom client config
   - Updated `reconnect()` to handle TLS connections (returns error for TLS)

### Error Handling
Added new error variants:
- `ClientError::Tls` - TLS handshake and connection errors
- `ClientError::InvalidCertificate` - Certificate loading/parsing errors
- `ClientError::InvalidDomain` - Invalid domain name errors

### Tests
Added comprehensive tests:
- TlsClientConfig creation and builder pattern
- Invalid CA certificate path handling
- Invalid domain handling
- ConnectionStream enum existence
- All existing tests still pass (27 total tests)

## Status
✅ Completed

## Notes
- TLS reconnection is not supported - users must create a new client
- Custom CA certificates can be loaded from PEM files
- Skip verification mode is available but marked as dangerous
- No webpki-roots dependency added (as per requirements)
