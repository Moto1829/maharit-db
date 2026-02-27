//! Lightweight HTTP server for metrics and health endpoints.
//!
//! Provides:
//! - `/metrics` - Prometheus text format metrics
//! - `/health` - JSON health check (liveness + readiness + custom checks)
//! - `/health/live` - Liveness probe (is the process alive?)
//! - `/health/ready` - Readiness probe (is the server accepting queries?)

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::metrics::Metrics;

/// Configuration for the HTTP monitoring server.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Address to bind the HTTP server to.
    pub bind_address: String,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0:9090".to_string(),
        }
    }
}

/// Shared state for health checks.
pub struct HealthState {
    /// Whether the server is ready to accept queries.
    pub ready: AtomicBool,
}

impl Default for HealthState {
    fn default() -> Self {
        Self {
            ready: AtomicBool::new(true),
        }
    }
}

impl HealthState {
    pub fn set_ready(&self, ready: bool) {
        self.ready.store(ready, Ordering::SeqCst);
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// Custom health check registry
// ---------------------------------------------------------------------------

/// The result of a single named health check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    /// The check passed.
    Healthy,
    /// The check failed with an optional description of the problem.
    Unhealthy(String),
}

impl HealthStatus {
    /// Return the JSON-safe string value used in the `/health` response.
    fn as_json_value(&self) -> String {
        match self {
            HealthStatus::Healthy => "healthy".to_string(),
            HealthStatus::Unhealthy(msg) => {
                // Escape double-quotes so the string is embeddable in JSON.
                format!("unhealthy: {}", msg.replace('"', "\\\""))
            }
        }
    }

    /// Return `true` if the status is [`HealthStatus::Healthy`].
    pub fn is_healthy(&self) -> bool {
        matches!(self, HealthStatus::Healthy)
    }
}

/// A registry of named health-check functions.
///
/// Register closures with [`HealthRegistry::register`]; the HTTP server will
/// call them on every `/health` request and include their results in the JSON
/// response.
///
/// # Example
///
/// ```
/// use maharit_server::http_server::{HealthRegistry, HealthStatus};
///
/// let registry = HealthRegistry::new();
/// registry.register("disk_space", || {
///     // Replace with real disk-space check.
///     HealthStatus::Healthy
/// });
///
/// let results = registry.run_checks();
/// assert_eq!(results["disk_space"], HealthStatus::Healthy);
/// ```
pub struct HealthRegistry {
    /// Named check closures.  `Box<dyn Fn() -> HealthStatus + Send + Sync>`
    /// allows arbitrary check logic without generics in the public API.
    checks: Mutex<Vec<(String, Box<dyn Fn() -> HealthStatus + Send + Sync>)>>,
}

impl std::fmt::Debug for HealthRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let guard = self.checks.lock().unwrap_or_else(|e| e.into_inner());
        let names: Vec<&str> = guard.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("HealthRegistry")
            .field("checks", &names)
            .finish()
    }
}

impl Default for HealthRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl HealthRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            checks: Mutex::new(Vec::new()),
        }
    }

    /// Register a named health-check function.
    ///
    /// The closure is called on every `/health` HTTP request.  It must be
    /// `Send + Sync + 'static` so it can be shared across async tasks.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn register<F>(&self, name: impl Into<String>, check: F)
    where
        F: Fn() -> HealthStatus + Send + Sync + 'static,
    {
        let mut guard = self.checks.lock().unwrap_or_else(|e| e.into_inner());
        guard.push((name.into(), Box::new(check)));
    }

    /// Run all registered checks and return a map of name → status.
    ///
    /// The map is sorted by name for deterministic JSON output.
    pub fn run_checks(&self) -> BTreeMap<String, HealthStatus> {
        let guard = self.checks.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .iter()
            .map(|(name, check)| (name.clone(), check()))
            .collect()
    }

    /// Return `true` if there are no registered checks, or if every registered
    /// check returns [`HealthStatus::Healthy`].
    pub fn all_healthy(&self) -> bool {
        let guard = self.checks.lock().unwrap_or_else(|e| e.into_inner());
        guard.iter().all(|(_, check)| check().is_healthy())
    }
}

/// Lightweight HTTP server for monitoring endpoints.
pub struct HttpServer {
    config: HttpConfig,
    metrics: Arc<Metrics>,
    health: Arc<HealthState>,
    /// Optional custom health-check registry.
    health_registry: Option<Arc<HealthRegistry>>,
}

impl HttpServer {
    /// Create a new HTTP server without custom health checks.
    pub fn new(config: HttpConfig, metrics: Arc<Metrics>, health: Arc<HealthState>) -> Self {
        Self {
            config,
            metrics,
            health,
            health_registry: None,
        }
    }

    /// Create a new HTTP server with a [`HealthRegistry`] for custom health checks.
    ///
    /// Checks registered in `registry` will be called on every `/health` request
    /// and their results included in the JSON response.
    pub fn with_health_registry(
        config: HttpConfig,
        metrics: Arc<Metrics>,
        health: Arc<HealthState>,
        registry: Arc<HealthRegistry>,
    ) -> Self {
        Self {
            config,
            metrics,
            health,
            health_registry: Some(registry),
        }
    }

    /// Start the HTTP server. This runs until the shutdown signal is received.
    pub async fn start(&self, shutdown: Arc<AtomicBool>) -> std::io::Result<()> {
        let listener = TcpListener::bind(&self.config.bind_address).await?;
        println!(
            "HTTP monitoring server listening on {}",
            self.config.bind_address
        );

        loop {
            if shutdown.load(Ordering::SeqCst) {
                break;
            }

            let accept_result =
                tokio::time::timeout(std::time::Duration::from_secs(1), listener.accept()).await;

            match accept_result {
                Ok(Ok((mut socket, _addr))) => {
                    let metrics = Arc::clone(&self.metrics);
                    let health = Arc::clone(&self.health);
                    let registry = self.health_registry.clone();

                    tokio::spawn(async move {
                        if let Err(_e) =
                            handle_http_request(&mut socket, &metrics, &health, registry.as_deref())
                                .await
                        {
                            // Connection errors are expected (client disconnects, etc.)
                        }
                    });
                }
                Ok(Err(_e)) => {
                    // Accept error, continue
                }
                Err(_) => {
                    // Timeout, check shutdown flag
                }
            }
        }

        Ok(())
    }
}

/// Serialize a `BTreeMap<String, HealthStatus>` as a JSON object fragment.
///
/// Returns `""` (empty string) when the map is empty so the caller can
/// conditionally include it in the parent object.
fn checks_to_json(checks: &BTreeMap<String, HealthStatus>) -> String {
    if checks.is_empty() {
        return String::new();
    }
    let inner: Vec<String> = checks
        .iter()
        .map(|(name, status)| format!("\"{}\":\"{}\"", name, status.as_json_value()))
        .collect();
    format!(",\"checks\":{{{}}}", inner.join(","))
}

/// Parse the HTTP request and route to the appropriate handler.
async fn handle_http_request(
    socket: &mut tokio::net::TcpStream,
    metrics: &Metrics,
    health: &HealthState,
    health_registry: Option<&HealthRegistry>,
) -> std::io::Result<()> {
    let mut buf = [0u8; 1024];
    let n = socket.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }

    let request = String::from_utf8_lossy(&buf[..n]);

    // Extract the request path from the first line: "GET /path HTTP/1.1"
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    let (status, content_type, body) = match path {
        "/metrics" => {
            let body = metrics.to_prometheus();
            ("200 OK", "text/plain; version=0.0.4; charset=utf-8", body)
        }
        "/health" => {
            let ready = health.is_ready();

            // Run custom checks (if any).
            let custom_checks = health_registry
                .map(|r| r.run_checks())
                .unwrap_or_default();
            let all_custom_healthy = custom_checks.values().all(|s| s.is_healthy());

            // Overall status: degraded if readiness probe fails, unhealthy if any custom
            // check fails, otherwise healthy.
            let overall_status = if !ready {
                "degraded"
            } else if !all_custom_healthy {
                "unhealthy"
            } else {
                "healthy"
            };

            let http_status_code = if ready && all_custom_healthy {
                "200 OK"
            } else {
                "503 Service Unavailable"
            };

            let checks_json = checks_to_json(&custom_checks);
            let body = format!(
                "{{\"status\":\"{}\",\"live\":true,\"ready\":{}{checks_json}}}",
                overall_status, ready,
            );
            (http_status_code, "application/json", body)
        }
        "/health/live" => {
            let body = "{\"live\":true}".to_string();
            ("200 OK", "application/json", body)
        }
        "/health/ready" => {
            let ready = health.is_ready();
            let status_code = if ready {
                "200 OK"
            } else {
                "503 Service Unavailable"
            };
            let body = format!("{{\"ready\":{}}}", ready);
            (status_code, "application/json", body)
        }
        _ => {
            let body = "Not Found".to_string();
            ("404 Not Found", "text/plain", body)
        }
    };

    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        content_type,
        body.len(),
        body
    );

    socket.write_all(response.as_bytes()).await?;
    socket.flush().await?;

    Ok(())
}

/// Helper to make an HTTP GET request and return the response body.
/// Used in tests.
#[cfg(test)]
async fn http_get(addr: &str, path: &str) -> (u16, String) {
    use tokio::io::AsyncBufReadExt;
    use tokio::io::BufReader;

    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    let request = format!("GET {} HTTP/1.1\r\nHost: {}\r\n\r\n", path, addr);
    socket.write_all(request.as_bytes()).await.unwrap();
    socket.flush().await.unwrap();

    // Read response
    let mut reader = BufReader::new(&mut socket);
    let mut status_line = String::new();
    reader.read_line(&mut status_line).await.unwrap();

    // Parse status code
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Read headers until blank line
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        if line.trim().is_empty() {
            break;
        }
        if let Some(val) = line.strip_prefix("Content-Length: ") {
            content_length = val.trim().parse().unwrap_or(0);
        }
    }

    // Read body
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).await.unwrap();

    (status_code, String::from_utf8_lossy(&body).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Find a free port by binding to port 0.
    async fn free_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().port()
    }

    /// Start an HttpServer in the background and return its address.
    async fn start_test_server(
        metrics: Arc<Metrics>,
        health: Arc<HealthState>,
    ) -> (String, Arc<AtomicBool>) {
        let port = free_port().await;
        let addr = format!("127.0.0.1:{}", port);
        let config = HttpConfig {
            bind_address: addr.clone(),
        };
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);

        let server = HttpServer::new(config, metrics, health);
        tokio::spawn(async move {
            let _ = server.start(shutdown_clone).await;
        });

        // Give the server time to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        (addr, shutdown)
    }

    #[tokio::test]
    async fn test_metrics_endpoint() {
        let metrics = Arc::new(Metrics::new());
        metrics.record_query(
            crate::metrics::QueryType::Create,
            Duration::from_micros(100),
        );
        metrics.set_node_count(42);
        metrics.set_edge_count(10);

        let health = Arc::new(HealthState::default());
        let (addr, shutdown) = start_test_server(metrics, health).await;

        let (status, body) = http_get(&addr, "/metrics").await;
        assert_eq!(status, 200);
        assert!(body.contains("maharit_queries_total{type=\"create\"} 1"));
        assert!(body.contains("maharit_nodes_total 42"));
        assert!(body.contains("maharit_edges_total 10"));
        assert!(body.contains("maharit_memory_usage_bytes"));
        assert!(body.contains("maharit_uptime_seconds"));

        shutdown.store(true, Ordering::SeqCst);
    }

    #[tokio::test]
    async fn test_health_endpoint_healthy() {
        let metrics = Arc::new(Metrics::new());
        let health = Arc::new(HealthState::default());
        let (addr, shutdown) = start_test_server(metrics, health).await;

        let (status, body) = http_get(&addr, "/health").await;
        assert_eq!(status, 200);
        assert!(body.contains("\"status\":\"healthy\""));
        assert!(body.contains("\"live\":true"));
        assert!(body.contains("\"ready\":true"));

        shutdown.store(true, Ordering::SeqCst);
    }

    #[tokio::test]
    async fn test_health_endpoint_degraded() {
        let metrics = Arc::new(Metrics::new());
        let health = Arc::new(HealthState::default());
        health.set_ready(false);

        let (addr, shutdown) = start_test_server(metrics, health).await;

        let (status, body) = http_get(&addr, "/health").await;
        assert_eq!(status, 503);
        assert!(body.contains("\"status\":\"degraded\""));
        assert!(body.contains("\"ready\":false"));

        shutdown.store(true, Ordering::SeqCst);
    }

    #[tokio::test]
    async fn test_health_live_endpoint() {
        let metrics = Arc::new(Metrics::new());
        let health = Arc::new(HealthState::default());
        let (addr, shutdown) = start_test_server(metrics, health).await;

        let (status, body) = http_get(&addr, "/health/live").await;
        assert_eq!(status, 200);
        assert!(body.contains("\"live\":true"));

        shutdown.store(true, Ordering::SeqCst);
    }

    #[tokio::test]
    async fn test_health_ready_endpoint() {
        let metrics = Arc::new(Metrics::new());
        let health = Arc::new(HealthState::default());
        let (addr, shutdown) = start_test_server(metrics, health).await;

        let (status, body) = http_get(&addr, "/health/ready").await;
        assert_eq!(status, 200);
        assert!(body.contains("\"ready\":true"));

        shutdown.store(true, Ordering::SeqCst);
    }

    #[tokio::test]
    async fn test_health_ready_not_ready() {
        let metrics = Arc::new(Metrics::new());
        let health = Arc::new(HealthState::default());
        health.set_ready(false);

        let (addr, shutdown) = start_test_server(metrics, health).await;

        let (status, body) = http_get(&addr, "/health/ready").await;
        assert_eq!(status, 503);
        assert!(body.contains("\"ready\":false"));

        shutdown.store(true, Ordering::SeqCst);
    }

    #[tokio::test]
    async fn test_not_found() {
        let metrics = Arc::new(Metrics::new());
        let health = Arc::new(HealthState::default());
        let (addr, shutdown) = start_test_server(metrics, health).await;

        let (status, body) = http_get(&addr, "/nonexistent").await;
        assert_eq!(status, 404);
        assert!(body.contains("Not Found"));

        shutdown.store(true, Ordering::SeqCst);
    }

    #[test]
    fn test_http_config_default() {
        let config = HttpConfig::default();
        assert_eq!(config.bind_address, "0.0.0.0:9090");
    }

    #[test]
    fn test_health_state_default() {
        let state = HealthState::default();
        assert!(state.is_ready());
    }

    #[test]
    fn test_health_state_toggle() {
        let state = HealthState::default();
        assert!(state.is_ready());

        state.set_ready(false);
        assert!(!state.is_ready());

        state.set_ready(true);
        assert!(state.is_ready());
    }

    // -----------------------------------------------------------------------
    // HealthRegistry unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_health_registry_empty_is_healthy() {
        let registry = HealthRegistry::new();
        assert!(registry.all_healthy());
        let checks = registry.run_checks();
        assert!(checks.is_empty());
    }

    #[test]
    fn test_health_registry_single_healthy_check() {
        let registry = HealthRegistry::new();
        registry.register("always_ok", || HealthStatus::Healthy);

        assert!(registry.all_healthy());
        let checks = registry.run_checks();
        assert_eq!(checks["always_ok"], HealthStatus::Healthy);
    }

    #[test]
    fn test_health_registry_single_unhealthy_check() {
        let registry = HealthRegistry::new();
        registry.register("disk_space", || {
            HealthStatus::Unhealthy("disk full".to_string())
        });

        assert!(!registry.all_healthy());
        let checks = registry.run_checks();
        assert_eq!(
            checks["disk_space"],
            HealthStatus::Unhealthy("disk full".to_string())
        );
    }

    #[test]
    fn test_health_registry_mixed_checks() {
        let registry = HealthRegistry::new();
        registry.register("ok_check", || HealthStatus::Healthy);
        registry.register("bad_check", || {
            HealthStatus::Unhealthy("something wrong".to_string())
        });

        assert!(!registry.all_healthy());
    }

    #[test]
    fn test_health_status_as_json_value() {
        assert_eq!(HealthStatus::Healthy.as_json_value(), "healthy");
        assert_eq!(
            HealthStatus::Unhealthy("disk full".to_string()).as_json_value(),
            "unhealthy: disk full"
        );
        // Test quote escaping
        assert_eq!(
            HealthStatus::Unhealthy("has \"quote\"".to_string()).as_json_value(),
            "unhealthy: has \\\"quote\\\""
        );
    }

    #[test]
    fn test_checks_to_json_empty() {
        let checks = BTreeMap::new();
        assert_eq!(checks_to_json(&checks), "");
    }

    #[test]
    fn test_checks_to_json_single() {
        let mut checks = BTreeMap::new();
        checks.insert("disk_space".to_string(), HealthStatus::Healthy);
        let json = checks_to_json(&checks);
        assert!(json.contains("\"disk_space\":\"healthy\""));
        assert!(json.starts_with(",\"checks\":{"));
    }

    // -----------------------------------------------------------------------
    // Integration tests: /health endpoint with HealthRegistry
    // -----------------------------------------------------------------------

    /// Start a server with a custom registry.
    async fn start_test_server_with_registry(
        metrics: Arc<Metrics>,
        health: Arc<HealthState>,
        registry: Arc<HealthRegistry>,
    ) -> (String, Arc<AtomicBool>) {
        let port = free_port().await;
        let addr = format!("127.0.0.1:{}", port);
        let config = HttpConfig {
            bind_address: addr.clone(),
        };
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);

        let server = HttpServer::with_health_registry(config, metrics, health, registry);
        tokio::spawn(async move {
            let _ = server.start(shutdown_clone).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        (addr, shutdown)
    }

    #[tokio::test]
    async fn test_health_endpoint_with_healthy_custom_check() {
        let metrics = Arc::new(Metrics::new());
        let health = Arc::new(HealthState::default());
        let registry = Arc::new(HealthRegistry::new());
        registry.register("disk_space", || HealthStatus::Healthy);

        let (addr, shutdown) =
            start_test_server_with_registry(metrics, health, registry).await;

        let (status, body) = http_get(&addr, "/health").await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"status\":\"healthy\""), "body: {body}");
        assert!(
            body.contains("\"checks\""),
            "checks key missing: {body}"
        );
        assert!(
            body.contains("\"disk_space\":\"healthy\""),
            "check result missing: {body}"
        );

        shutdown.store(true, Ordering::SeqCst);
    }

    #[tokio::test]
    async fn test_health_endpoint_with_unhealthy_custom_check() {
        let metrics = Arc::new(Metrics::new());
        let health = Arc::new(HealthState::default());
        let registry = Arc::new(HealthRegistry::new());
        registry.register("disk_space", || {
            HealthStatus::Unhealthy("disk full".to_string())
        });

        let (addr, shutdown) =
            start_test_server_with_registry(metrics, health, registry).await;

        let (status, body) = http_get(&addr, "/health").await;
        // Readiness is OK but custom check fails → 503 + unhealthy status.
        assert_eq!(status, 503, "body: {body}");
        assert!(body.contains("\"status\":\"unhealthy\""), "body: {body}");
        assert!(
            body.contains("\"disk_space\":\"unhealthy: disk full\""),
            "check result missing: {body}"
        );

        shutdown.store(true, Ordering::SeqCst);
    }

    #[tokio::test]
    async fn test_health_endpoint_multiple_checks_all_healthy() {
        let metrics = Arc::new(Metrics::new());
        let health = Arc::new(HealthState::default());
        let registry = Arc::new(HealthRegistry::new());
        registry.register("check_a", || HealthStatus::Healthy);
        registry.register("check_b", || HealthStatus::Healthy);

        let (addr, shutdown) =
            start_test_server_with_registry(metrics, health, registry).await;

        let (status, body) = http_get(&addr, "/health").await;
        assert_eq!(status, 200, "body: {body}");
        assert!(body.contains("\"status\":\"healthy\""), "body: {body}");
        assert!(body.contains("\"check_a\":\"healthy\""), "body: {body}");
        assert!(body.contains("\"check_b\":\"healthy\""), "body: {body}");

        shutdown.store(true, Ordering::SeqCst);
    }

    #[tokio::test]
    async fn test_health_endpoint_readiness_false_takes_priority() {
        let metrics = Arc::new(Metrics::new());
        let health = Arc::new(HealthState::default());
        health.set_ready(false);
        let registry = Arc::new(HealthRegistry::new());
        registry.register("always_ok", || HealthStatus::Healthy);

        let (addr, shutdown) =
            start_test_server_with_registry(metrics, health, registry).await;

        let (status, body) = http_get(&addr, "/health").await;
        assert_eq!(status, 503, "body: {body}");
        assert!(body.contains("\"status\":\"degraded\""), "body: {body}");

        shutdown.store(true, Ordering::SeqCst);
    }
}
