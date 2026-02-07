//! Lightweight metrics collection for the database server.
//!
//! This module provides thread-safe metrics tracking for queries, errors, latencies,
//! and resource counts. Metrics can be exported in Prometheus text format.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Query type for tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueryType {
    Create,
    Match,
    Delete,
    Set,
    Merge,
    Remove,
    Unwind,
    Constraint,
    Other,
}

impl QueryType {
    /// Convert query type to array index.
    const fn to_index(self) -> usize {
        match self {
            QueryType::Create => 0,
            QueryType::Match => 1,
            QueryType::Delete => 2,
            QueryType::Set => 3,
            QueryType::Merge => 4,
            QueryType::Remove => 5,
            QueryType::Unwind => 6,
            QueryType::Constraint => 7,
            QueryType::Other => 8,
        }
    }

    /// Convert query type to lowercase string label for Prometheus.
    const fn as_str(self) -> &'static str {
        match self {
            QueryType::Create => "create",
            QueryType::Match => "match",
            QueryType::Delete => "delete",
            QueryType::Set => "set",
            QueryType::Merge => "merge",
            QueryType::Remove => "remove",
            QueryType::Unwind => "unwind",
            QueryType::Constraint => "constraint",
            QueryType::Other => "other",
        }
    }

    /// All query type variants.
    const ALL: [QueryType; 9] = [
        QueryType::Create,
        QueryType::Match,
        QueryType::Delete,
        QueryType::Set,
        QueryType::Merge,
        QueryType::Remove,
        QueryType::Unwind,
        QueryType::Constraint,
        QueryType::Other,
    ];
}

/// Latency percentile statistics.
#[derive(Debug, Clone, Copy)]
pub struct LatencyPercentiles {
    /// 50th percentile (median) in microseconds.
    pub p50: u64,
    /// 95th percentile in microseconds.
    pub p95: u64,
    /// 99th percentile in microseconds.
    pub p99: u64,
}

/// Ring buffer for tracking recent latency samples.
struct LatencyTracker {
    /// Ring buffer of recent latency samples (microseconds).
    samples: Vec<u64>,
    /// Maximum number of samples to keep.
    max_samples: usize,
    /// Current write position in the ring buffer.
    write_pos: usize,
    /// Total number of samples recorded (may exceed max_samples).
    count: usize,
}

impl LatencyTracker {
    fn new(max_samples: usize) -> Self {
        Self {
            samples: Vec::with_capacity(max_samples),
            max_samples,
            write_pos: 0,
            count: 0,
        }
    }

    fn record(&mut self, duration: Duration) {
        let micros = duration.as_micros() as u64;

        if self.samples.len() < self.max_samples {
            self.samples.push(micros);
        } else {
            self.samples[self.write_pos] = micros;
        }

        self.write_pos = (self.write_pos + 1) % self.max_samples;
        self.count += 1;
    }

    fn percentiles(&self) -> LatencyPercentiles {
        if self.samples.is_empty() {
            return LatencyPercentiles {
                p50: 0,
                p95: 0,
                p99: 0,
            };
        }

        // Create a sorted copy of the samples
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();

        let len = sorted.len();
        let p50_idx = (len as f64 * 0.50) as usize;
        let p95_idx = (len as f64 * 0.95) as usize;
        let p99_idx = (len as f64 * 0.99) as usize;

        LatencyPercentiles {
            p50: sorted[p50_idx.min(len - 1)],
            p95: sorted[p95_idx.min(len - 1)],
            p99: sorted[p99_idx.min(len - 1)],
        }
    }

    fn reset(&mut self) {
        self.samples.clear();
        self.write_pos = 0;
        self.count = 0;
    }
}

/// Metrics collector for the database server.
///
/// This struct is thread-safe and can be shared across threads using `Arc<Metrics>`.
pub struct Metrics {
    /// Query counters per type.
    query_counts: [AtomicU64; 9],
    /// Total query count across all types.
    total_queries: AtomicU64,
    /// Total error count.
    error_count: AtomicU64,
    /// Latency samples for percentile calculation.
    latencies: Mutex<LatencyTracker>,
    /// Current number of nodes (gauge).
    node_count: AtomicI64,
    /// Current number of edges (gauge).
    edge_count: AtomicI64,
    /// Current number of active connections (gauge).
    current_connections: AtomicI64,
    /// Total number of connections ever created (counter).
    total_connections: AtomicU64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    /// Create a new metrics collector with default settings.
    pub fn new() -> Self {
        Self::with_latency_samples(10000)
    }

    /// Create a new metrics collector with a custom number of latency samples.
    pub fn with_latency_samples(max_samples: usize) -> Self {
        Self {
            query_counts: [
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ],
            total_queries: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            latencies: Mutex::new(LatencyTracker::new(max_samples)),
            node_count: AtomicI64::new(0),
            edge_count: AtomicI64::new(0),
            current_connections: AtomicI64::new(0),
            total_connections: AtomicU64::new(0),
        }
    }

    /// Record a query execution.
    pub fn record_query(&self, query_type: QueryType, duration: Duration) {
        let idx = query_type.to_index();
        self.query_counts[idx].fetch_add(1, Ordering::Relaxed);
        self.total_queries.fetch_add(1, Ordering::Relaxed);

        if let Ok(mut latencies) = self.latencies.lock() {
            latencies.record(duration);
        }
    }

    /// Start timing a query. The duration will be automatically recorded when the timer is dropped.
    pub fn start_query(&self, query_type: QueryType) -> QueryTimer<'_> {
        QueryTimer {
            metrics: self,
            query_type,
            start: Instant::now(),
        }
    }

    /// Record an error.
    pub fn record_error(&self) {
        self.error_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Set the current node count.
    pub fn set_node_count(&self, count: i64) {
        self.node_count.store(count, Ordering::Relaxed);
    }

    /// Set the current edge count.
    pub fn set_edge_count(&self, count: i64) {
        self.edge_count.store(count, Ordering::Relaxed);
    }

    /// Increment the connection count (both current and total).
    pub fn increment_connections(&self) {
        self.current_connections.fetch_add(1, Ordering::Relaxed);
        self.total_connections.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement the current connection count.
    pub fn decrement_connections(&self) {
        self.current_connections.fetch_sub(1, Ordering::Relaxed);
    }

    /// Get the query count for a specific type.
    pub fn query_count(&self, query_type: QueryType) -> u64 {
        let idx = query_type.to_index();
        self.query_counts[idx].load(Ordering::Relaxed)
    }

    /// Get the total query count across all types.
    pub fn total_query_count(&self) -> u64 {
        self.total_queries.load(Ordering::Relaxed)
    }

    /// Get the error count.
    pub fn error_count(&self) -> u64 {
        self.error_count.load(Ordering::Relaxed)
    }

    /// Get the current node count.
    pub fn node_count(&self) -> i64 {
        self.node_count.load(Ordering::Relaxed)
    }

    /// Get the current edge count.
    pub fn edge_count(&self) -> i64 {
        self.edge_count.load(Ordering::Relaxed)
    }

    /// Get the current connection count.
    pub fn current_connections(&self) -> i64 {
        self.current_connections.load(Ordering::Relaxed)
    }

    /// Get the total connection count.
    pub fn total_connections(&self) -> u64 {
        self.total_connections.load(Ordering::Relaxed)
    }

    /// Calculate latency percentiles from recorded samples.
    pub fn latency_percentiles(&self) -> LatencyPercentiles {
        self.latencies
            .lock()
            .map(|tracker| tracker.percentiles())
            .unwrap_or(LatencyPercentiles {
                p50: 0,
                p95: 0,
                p99: 0,
            })
    }

    /// Export metrics in Prometheus text format.
    pub fn to_prometheus(&self) -> String {
        let mut output = String::new();

        // Query counts by type
        output.push_str("# HELP maharit_queries_total Total number of queries executed\n");
        output.push_str("# TYPE maharit_queries_total counter\n");
        for query_type in QueryType::ALL {
            let count = self.query_count(query_type);
            output.push_str(&format!(
                "maharit_queries_total{{type=\"{}\"}} {}\n",
                query_type.as_str(),
                count
            ));
        }

        // Error count
        output.push_str("# HELP maharit_errors_total Total number of errors\n");
        output.push_str("# TYPE maharit_errors_total counter\n");
        output.push_str(&format!("maharit_errors_total {}\n", self.error_count()));

        // Latency percentiles
        let percentiles = self.latency_percentiles();
        output.push_str("# HELP maharit_query_latency_microseconds Query latency percentiles\n");
        output.push_str("# TYPE maharit_query_latency_microseconds summary\n");
        output.push_str(&format!(
            "maharit_query_latency_microseconds{{quantile=\"0.5\"}} {}\n",
            percentiles.p50
        ));
        output.push_str(&format!(
            "maharit_query_latency_microseconds{{quantile=\"0.95\"}} {}\n",
            percentiles.p95
        ));
        output.push_str(&format!(
            "maharit_query_latency_microseconds{{quantile=\"0.99\"}} {}\n",
            percentiles.p99
        ));

        // Node count
        output.push_str("# HELP maharit_nodes_total Current number of nodes\n");
        output.push_str("# TYPE maharit_nodes_total gauge\n");
        output.push_str(&format!("maharit_nodes_total {}\n", self.node_count()));

        // Edge count
        output.push_str("# HELP maharit_edges_total Current number of edges\n");
        output.push_str("# TYPE maharit_edges_total gauge\n");
        output.push_str(&format!("maharit_edges_total {}\n", self.edge_count()));

        // Current connections
        output.push_str("# HELP maharit_connections_current Current number of connections\n");
        output.push_str("# TYPE maharit_connections_current gauge\n");
        output.push_str(&format!(
            "maharit_connections_current {}\n",
            self.current_connections()
        ));

        // Total connections
        output.push_str("# HELP maharit_connections_total Total number of connections\n");
        output.push_str("# TYPE maharit_connections_total counter\n");
        output.push_str(&format!(
            "maharit_connections_total {}\n",
            self.total_connections()
        ));

        output
    }

    /// Reset all metrics to zero. Useful for testing.
    pub fn reset(&self) {
        for counter in &self.query_counts {
            counter.store(0, Ordering::Relaxed);
        }
        self.total_queries.store(0, Ordering::Relaxed);
        self.error_count.store(0, Ordering::Relaxed);
        self.node_count.store(0, Ordering::Relaxed);
        self.edge_count.store(0, Ordering::Relaxed);
        self.current_connections.store(0, Ordering::Relaxed);
        self.total_connections.store(0, Ordering::Relaxed);

        if let Ok(mut latencies) = self.latencies.lock() {
            latencies.reset();
        }
    }
}

/// Timer that automatically records query duration on drop.
pub struct QueryTimer<'a> {
    metrics: &'a Metrics,
    query_type: QueryType,
    start: Instant,
}

impl Drop for QueryTimer<'_> {
    fn drop(&mut self) {
        self.metrics
            .record_query(self.query_type, self.start.elapsed());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_new_metrics_zero() {
        let metrics = Metrics::new();
        assert_eq!(metrics.total_query_count(), 0);
        assert_eq!(metrics.error_count(), 0);
        assert_eq!(metrics.node_count(), 0);
        assert_eq!(metrics.edge_count(), 0);
        assert_eq!(metrics.current_connections(), 0);
        assert_eq!(metrics.total_connections(), 0);

        for query_type in QueryType::ALL {
            assert_eq!(metrics.query_count(query_type), 0);
        }
    }

    #[test]
    fn test_record_query() {
        let metrics = Metrics::new();

        metrics.record_query(QueryType::Create, Duration::from_micros(100));
        assert_eq!(metrics.query_count(QueryType::Create), 1);
        assert_eq!(metrics.total_query_count(), 1);

        metrics.record_query(QueryType::Match, Duration::from_micros(200));
        assert_eq!(metrics.query_count(QueryType::Match), 1);
        assert_eq!(metrics.total_query_count(), 2);

        metrics.record_query(QueryType::Create, Duration::from_micros(150));
        assert_eq!(metrics.query_count(QueryType::Create), 2);
        assert_eq!(metrics.total_query_count(), 3);
    }

    #[test]
    fn test_record_error() {
        let metrics = Metrics::new();

        assert_eq!(metrics.error_count(), 0);

        metrics.record_error();
        assert_eq!(metrics.error_count(), 1);

        metrics.record_error();
        assert_eq!(metrics.error_count(), 2);
    }

    #[test]
    fn test_node_edge_gauges() {
        let metrics = Metrics::new();

        metrics.set_node_count(1000);
        assert_eq!(metrics.node_count(), 1000);

        metrics.set_edge_count(5000);
        assert_eq!(metrics.edge_count(), 5000);

        metrics.set_node_count(2000);
        assert_eq!(metrics.node_count(), 2000);
    }

    #[test]
    fn test_connections() {
        let metrics = Metrics::new();

        assert_eq!(metrics.current_connections(), 0);
        assert_eq!(metrics.total_connections(), 0);

        metrics.increment_connections();
        assert_eq!(metrics.current_connections(), 1);
        assert_eq!(metrics.total_connections(), 1);

        metrics.increment_connections();
        assert_eq!(metrics.current_connections(), 2);
        assert_eq!(metrics.total_connections(), 2);

        metrics.decrement_connections();
        assert_eq!(metrics.current_connections(), 1);
        assert_eq!(metrics.total_connections(), 2);

        metrics.increment_connections();
        assert_eq!(metrics.current_connections(), 2);
        assert_eq!(metrics.total_connections(), 3);
    }

    #[test]
    fn test_latency_percentiles() {
        let metrics = Metrics::with_latency_samples(100);

        // Record some latencies
        metrics.record_query(QueryType::Match, Duration::from_micros(100));
        metrics.record_query(QueryType::Match, Duration::from_micros(200));
        metrics.record_query(QueryType::Match, Duration::from_micros(300));
        metrics.record_query(QueryType::Match, Duration::from_micros(400));
        metrics.record_query(QueryType::Match, Duration::from_micros(500));

        let percentiles = metrics.latency_percentiles();
        assert!(percentiles.p50 >= 100);
        assert!(percentiles.p50 <= 500);
        assert!(percentiles.p95 >= percentiles.p50);
        assert!(percentiles.p99 >= percentiles.p95);
    }

    #[test]
    fn test_query_timer() {
        let metrics = Metrics::new();

        {
            let _timer = metrics.start_query(QueryType::Create);
            thread::sleep(Duration::from_micros(100));
        }

        assert_eq!(metrics.query_count(QueryType::Create), 1);
        assert_eq!(metrics.total_query_count(), 1);

        let percentiles = metrics.latency_percentiles();
        assert!(percentiles.p50 >= 100);
    }

    #[test]
    fn test_to_prometheus() {
        let metrics = Metrics::new();

        metrics.record_query(QueryType::Create, Duration::from_micros(100));
        metrics.record_query(QueryType::Match, Duration::from_micros(200));
        metrics.record_error();
        metrics.set_node_count(1000);
        metrics.set_edge_count(5000);
        metrics.increment_connections();

        let output = metrics.to_prometheus();

        // Check for key strings in the output
        assert!(output.contains("maharit_queries_total{type=\"create\"} 1"));
        assert!(output.contains("maharit_queries_total{type=\"match\"} 1"));
        assert!(output.contains("maharit_errors_total 1"));
        assert!(output.contains("maharit_nodes_total 1000"));
        assert!(output.contains("maharit_edges_total 5000"));
        assert!(output.contains("maharit_connections_current 1"));
        assert!(output.contains("maharit_connections_total 1"));
        assert!(output.contains("maharit_query_latency_microseconds{quantile=\"0.5\"}"));
        assert!(output.contains("maharit_query_latency_microseconds{quantile=\"0.95\"}"));
        assert!(output.contains("maharit_query_latency_microseconds{quantile=\"0.99\"}"));
    }

    #[test]
    fn test_reset() {
        let metrics = Metrics::new();

        metrics.record_query(QueryType::Create, Duration::from_micros(100));
        metrics.record_error();
        metrics.set_node_count(1000);
        metrics.increment_connections();

        assert_eq!(metrics.total_query_count(), 1);
        assert_eq!(metrics.error_count(), 1);
        assert_eq!(metrics.node_count(), 1000);
        assert_eq!(metrics.current_connections(), 1);

        metrics.reset();

        assert_eq!(metrics.total_query_count(), 0);
        assert_eq!(metrics.error_count(), 0);
        assert_eq!(metrics.node_count(), 0);
        assert_eq!(metrics.current_connections(), 0);
        assert_eq!(metrics.total_connections(), 0);

        let percentiles = metrics.latency_percentiles();
        assert_eq!(percentiles.p50, 0);
        assert_eq!(percentiles.p95, 0);
        assert_eq!(percentiles.p99, 0);
    }

    #[test]
    fn test_thread_safety() {
        let metrics = Arc::new(Metrics::new());
        let mut handles = vec![];

        for i in 0..10 {
            let metrics = Arc::clone(&metrics);
            let handle = thread::spawn(move || {
                let query_type = if i % 2 == 0 {
                    QueryType::Create
                } else {
                    QueryType::Match
                };
                metrics.record_query(query_type, Duration::from_micros(100 * i));
                metrics.record_error();
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(metrics.total_query_count(), 10);
        assert_eq!(metrics.error_count(), 10);
    }
}
