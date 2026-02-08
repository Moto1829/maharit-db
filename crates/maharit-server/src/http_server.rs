//! Lightweight HTTP server for metrics and health endpoints.
//!
//! Provides:
//! - `/metrics` - Prometheus text format metrics
//! - `/health` - JSON health check (liveness + readiness)
//! - `/health/live` - Liveness probe (is the process alive?)
//! - `/health/ready` - Readiness probe (is the server accepting queries?)

use std::sync::Arc;
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

/// Lightweight HTTP server for monitoring endpoints.
pub struct HttpServer {
    config: HttpConfig,
    metrics: Arc<Metrics>,
    health: Arc<HealthState>,
}

impl HttpServer {
    /// Create a new HTTP server.
    pub fn new(config: HttpConfig, metrics: Arc<Metrics>, health: Arc<HealthState>) -> Self {
        Self {
            config,
            metrics,
            health,
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

                    tokio::spawn(async move {
                        if let Err(_e) = handle_http_request(&mut socket, &metrics, &health).await
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

/// Parse the HTTP request and route to the appropriate handler.
async fn handle_http_request(
    socket: &mut tokio::net::TcpStream,
    metrics: &Metrics,
    health: &HealthState,
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
            (
                "200 OK",
                "text/plain; version=0.0.4; charset=utf-8",
                body,
            )
        }
        "/health" => {
            let ready = health.is_ready();
            let status_code = if ready { "200 OK" } else { "503 Service Unavailable" };
            let body = format!(
                "{{\"status\":\"{}\",\"live\":true,\"ready\":{}}}",
                if ready { "healthy" } else { "degraded" },
                ready
            );
            (status_code, "application/json", body)
        }
        "/health/live" => {
            let body = "{\"live\":true}".to_string();
            ("200 OK", "application/json", body)
        }
        "/health/ready" => {
            let ready = health.is_ready();
            let status_code = if ready { "200 OK" } else { "503 Service Unavailable" };
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
}
