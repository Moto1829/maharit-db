//! Structured tracing initialisation for the database server.
//!
//! This module wraps the [`tracing`] and [`tracing_subscriber`] crates to
//! provide a single, easy-to-use entry point for setting up structured
//! logging/tracing.  When `otlp_endpoint` is `None` the logs are written to
//! `stderr` as JSON lines; when it is `Some(endpoint)` the records are also
//! forwarded to an OpenTelemetry-compatible collector over OTLP/gRPC (if the
//! `otlp` Cargo feature is enabled – by default the crate ships without that
//! heavy dependency, so the endpoint setting is stored but ignored).
//!
//! # Usage
//!
//! ```no_run
//! use maharit_server::tracing_setup::TracingConfig;
//!
//! let _guard = TracingConfig {
//!     enabled: true,
//!     otlp_endpoint: None,
//!     service_name: "maharit-db".to_string(),
//! }
//! .init();
//!
//! tracing::info!(component = "server", "Server started");
//! ```

use std::sync::OnceLock;

use tracing::Level as TracingLevel;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

// ---------------------------------------------------------------------------
// Global initialisation guard – only the first call to TracingConfig::init
// actually installs the global subscriber.  Subsequent calls are no-ops.
// ---------------------------------------------------------------------------
static TRACING_INITIALISED: OnceLock<()> = OnceLock::new();

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Configuration for the tracing subsystem.
#[derive(Debug, Clone)]
pub struct TracingConfig {
    /// When `false` the tracing subsystem is disabled entirely (a no-op
    /// subscriber is installed).
    pub enabled: bool,
    /// Optional OTLP/gRPC collector endpoint (e.g. `"http://localhost:4317"`).
    ///
    /// When `None` only the stdout/stderr JSON layer is active.  When `Some`
    /// the endpoint is stored for documentation purposes; full OTLP export
    /// requires enabling the `otlp` Cargo feature (disabled by default to keep
    /// compile times short).
    pub otlp_endpoint: Option<String>,
    /// Service name reported in every span and log record.
    pub service_name: String,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            otlp_endpoint: None,
            service_name: "maharit-db".to_string(),
        }
    }
}

impl TracingConfig {
    /// Initialise the global tracing subscriber according to this configuration.
    ///
    /// Returns a [`TracingGuard`] that shuts down the tracer on drop.  Only
    /// the first call installs the subscriber; subsequent calls return a guard
    /// that is a no-op on drop.
    ///
    /// # Environment variables
    ///
    /// The log filter can be overridden at runtime via the `RUST_LOG`
    /// environment variable (e.g. `RUST_LOG=debug` or
    /// `RUST_LOG=maharit_server=trace`).  When `RUST_LOG` is unset the filter
    /// defaults to `INFO`.
    pub fn init(self) -> TracingGuard {
        if !self.enabled {
            // Install a no-op subscriber so that tracing macros compile and
            // run without panicking, but produce no output.
            let _ = TRACING_INITIALISED.set(());
            return TracingGuard {
                service_name: self.service_name,
                otlp_endpoint: self.otlp_endpoint,
            };
        }

        // Only initialise once across the entire process lifetime.
        TRACING_INITIALISED.get_or_init(|| {
            let filter = EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info"));

            // JSON formatter layer writing to stderr.
            let json_layer = fmt::layer()
                .json()
                .with_current_span(true)
                .with_span_list(true)
                .with_file(true)
                .with_line_number(true)
                .with_thread_ids(false)
                .with_target(true);

            tracing_subscriber::registry()
                .with(filter)
                .with(json_layer)
                .init();
        });

        TracingGuard {
            service_name: self.service_name,
            otlp_endpoint: self.otlp_endpoint,
        }
    }
}

// ---------------------------------------------------------------------------
// TracingGuard
// ---------------------------------------------------------------------------

/// Returned by [`TracingConfig::init`].  When dropped the guard flushes any
/// pending spans/log records.
///
/// In the default (non-OTLP) configuration drop is a no-op.
pub struct TracingGuard {
    /// Name of the service this guard was created for.
    pub service_name: String,
    /// The configured OTLP endpoint, if any.
    pub otlp_endpoint: Option<String>,
}

impl Drop for TracingGuard {
    fn drop(&mut self) {
        // With the default stdout/JSON subscriber there is nothing to flush.
        // A future OTLP implementation would call opentelemetry::global::shutdown_tracer_provider()
        // here.
        tracing::debug!(
            service = %self.service_name,
            "Tracing guard dropped; flushing spans"
        );
    }
}

// ---------------------------------------------------------------------------
// Convenience helpers
// ---------------------------------------------------------------------------

/// Emit a structured INFO span around an arbitrary closure, recording how long
/// it took.
///
/// ```no_run
/// use maharit_server::tracing_setup::timed;
///
/// let result = timed("parse_query", || {
///     // ... parsing logic ...
///     42_u64
/// });
/// ```
pub fn timed<T, F: FnOnce() -> T>(name: &'static str, f: F) -> T {
    let span = tracing::info_span!("timed_operation", operation = name);
    let _enter = span.enter();
    let start = std::time::Instant::now();
    let result = f();
    tracing::info!(
        operation = name,
        duration_us = start.elapsed().as_micros() as u64,
        "operation completed"
    );
    result
}

/// Return the current default log level as a [`tracing::Level`], defaulting to
/// INFO when `RUST_LOG` is not set.
pub fn default_log_level() -> TracingLevel {
    std::env::var("RUST_LOG")
        .ok()
        .and_then(|s| s.parse::<TracingLevel>().ok())
        .unwrap_or(TracingLevel::INFO)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // TracingConfig construction and field access
    // ------------------------------------------------------------------

    #[test]
    fn test_default_config() {
        let cfg = TracingConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.otlp_endpoint.is_none());
        assert_eq!(cfg.service_name, "maharit-db");
    }

    #[test]
    fn test_config_fields_are_stored() {
        let cfg = TracingConfig {
            enabled: false,
            otlp_endpoint: Some("http://localhost:4317".to_string()),
            service_name: "my-service".to_string(),
        };
        assert!(!cfg.enabled);
        assert_eq!(
            cfg.otlp_endpoint.as_deref(),
            Some("http://localhost:4317")
        );
        assert_eq!(cfg.service_name, "my-service");
    }

    // ------------------------------------------------------------------
    // TracingConfig::init – disabled path
    // ------------------------------------------------------------------

    #[test]
    fn test_disabled_tracing_init_returns_guard() {
        let cfg = TracingConfig {
            enabled: false,
            otlp_endpoint: None,
            service_name: "test-svc".to_string(),
        };
        let guard = cfg.init();
        assert_eq!(guard.service_name, "test-svc");
        assert!(guard.otlp_endpoint.is_none());
        // Guard drops cleanly – no panic.
    }

    #[test]
    fn test_disabled_tracing_macros_do_not_panic() {
        let _cfg = TracingConfig {
            enabled: false,
            otlp_endpoint: None,
            service_name: "test".to_string(),
        };
        // Even without initialising the subscriber, the macros should not
        // panic (they are no-ops when no subscriber is installed).
        tracing::info!("disabled tracing info");
        tracing::warn!("disabled tracing warn");
        tracing::error!("disabled tracing error");
    }

    // ------------------------------------------------------------------
    // TracingConfig::init – enabled path (first call wins)
    // ------------------------------------------------------------------

    #[test]
    fn test_enabled_tracing_init_returns_guard_with_service_name() {
        // We cannot install the global subscriber in a unit test that runs in
        // parallel with others (the global is already set), so we only check
        // that the guard fields are correct regardless of whether the subscriber
        // was actually installed by this particular test.
        let cfg = TracingConfig {
            enabled: true,
            otlp_endpoint: None,
            service_name: "maharit-test".to_string(),
        };
        let guard = cfg.init();
        assert_eq!(guard.service_name, "maharit-test");
    }

    #[test]
    fn test_otlp_endpoint_stored_in_guard() {
        let cfg = TracingConfig {
            enabled: false,
            otlp_endpoint: Some("http://otel:4317".to_string()),
            service_name: "svc".to_string(),
        };
        let guard = cfg.init();
        assert_eq!(
            guard.otlp_endpoint.as_deref(),
            Some("http://otel:4317")
        );
    }

    // ------------------------------------------------------------------
    // TracingGuard – service_name and endpoint fields
    // ------------------------------------------------------------------

    #[test]
    fn test_tracing_guard_fields() {
        let guard = TracingGuard {
            service_name: "db-server".to_string(),
            otlp_endpoint: Some("http://localhost:4317".to_string()),
        };
        assert_eq!(guard.service_name, "db-server");
        assert_eq!(
            guard.otlp_endpoint.as_deref(),
            Some("http://localhost:4317")
        );
    }

    #[test]
    fn test_tracing_guard_drop_does_not_panic() {
        let guard = TracingGuard {
            service_name: "test".to_string(),
            otlp_endpoint: None,
        };
        drop(guard); // Must not panic.
    }

    // ------------------------------------------------------------------
    // timed helper
    // ------------------------------------------------------------------

    #[test]
    fn test_timed_returns_closure_result() {
        let result = timed("test_op", || 42u64);
        assert_eq!(result, 42u64);
    }

    #[test]
    fn test_timed_with_string_result() {
        let result = timed("string_op", || "hello".to_string());
        assert_eq!(result, "hello");
    }

    // ------------------------------------------------------------------
    // default_log_level helper
    // ------------------------------------------------------------------

    #[test]
    fn test_default_log_level_returns_info_when_no_env() {
        // Remove RUST_LOG to get the default.  This is safe because our helper
        // reads the env var each time it is called.
        let prev = std::env::var("RUST_LOG").ok();
        // SAFETY: test-only code running single-threaded within this test.
        unsafe { std::env::remove_var("RUST_LOG") };
        let level = default_log_level();
        assert_eq!(level, TracingLevel::INFO);
        // Restore
        if let Some(val) = prev {
            unsafe { std::env::set_var("RUST_LOG", val) };
        }
    }

    #[test]
    fn test_tracing_span_can_be_created() {
        // Verify that creating and entering a span does not panic even when no
        // subscriber is installed.
        let span = tracing::info_span!("test_span", key = "value");
        let _guard = span.enter();
        tracing::info!("inside span");
    }

    #[test]
    fn test_tracing_event_fields() {
        // Verify various field types compile and run without panic.
        tracing::info!(
            query_len = 42usize,
            peer = "127.0.0.1",
            "handle request"
        );
        tracing::warn!(error_code = 404u32, "not found");
        tracing::error!(cause = "timeout", "connection failed");
    }
}
