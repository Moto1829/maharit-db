//! Shard coordinator server.
//!
//! Accepts client connections using the same wire protocol as [`TcpServer`]
//! (4-byte length-prefix + JSON) but instead of executing queries locally it
//! fans them out to the configured shard nodes via [`ShardClient`], merges
//! the results, and returns them to the client.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use maharit_cluster::{ClusterConfig, Row, RowValue, ShardId};
use maharit_cluster::shard_client::ShardClient;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::timeout;

// ─── Wire types (mirrors tcp_server) ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum CoordRequest {
    #[serde(rename = "query")]
    Query {
        query: String,
        #[serde(rename = "txId")]
        #[allow(dead_code)]
        tx_id: Option<u64>,
    },
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "stats")]
    Stats,
    #[serde(rename = "disconnect")]
    Disconnect,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum CoordResponse {
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
        shards: usize,
    },
    #[serde(rename = "goodbye")]
    Goodbye,
}

// ─── ShardPool ────────────────────────────────────────────────────────────────

/// A pool of [`ShardClient`]s keyed by shard ID, protected by a `Mutex`.
type ShardPool = Arc<HashMap<ShardId, Mutex<ShardClient>>>;

fn build_shard_pool(config: &ClusterConfig) -> ShardPool {
    let mut map = HashMap::new();
    for shard in &config.shards {
        map.insert(shard.id, Mutex::new(ShardClient::new(&shard.address)));
    }
    Arc::new(map)
}

// ─── ShardCoordinatorServer ───────────────────────────────────────────────────

/// Configuration for the coordinator server.
#[derive(Debug, Clone)]
pub struct CoordinatorConfig {
    /// Address to listen on (e.g. `"127.0.0.1:7690"`).
    pub bind_address: String,
    /// Maximum concurrent client connections.
    pub max_connections: usize,
    /// Per-connection read/write timeout.
    pub timeout: Duration,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:7690".to_string(),
            max_connections: 100,
            timeout: Duration::from_secs(30),
        }
    }
}

/// Coordinator server that fans queries out to shard nodes and merges results.
pub struct ShardCoordinatorServer {
    coord_config: CoordinatorConfig,
    pool: ShardPool,
    shard_ids: Vec<ShardId>,
    shutdown: Arc<AtomicBool>,
}

impl ShardCoordinatorServer {
    /// Create a new coordinator server from the given configurations.
    pub fn new(coord_config: CoordinatorConfig, cluster_config: ClusterConfig) -> Self {
        let shard_ids: Vec<ShardId> = cluster_config.shards.iter().map(|s| s.id).collect();
        let pool = build_shard_pool(&cluster_config);
        Self {
            coord_config,
            pool,
            shard_ids,
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Signal the server to stop accepting new connections.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    /// Start listening and serving client connections.
    pub async fn start(self: Arc<Self>) -> std::io::Result<()> {
        let listener = TcpListener::bind(&self.coord_config.bind_address).await?;
        println!(
            "Coordinator listening on {} ({} shards)",
            self.coord_config.bind_address,
            self.shard_ids.len()
        );

        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                break;
            }

            let (stream, peer) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    if self.shutdown.load(Ordering::Relaxed) {
                        break;
                    }
                    eprintln!("Coordinator: accept error: {}", e);
                    continue;
                }
            };

            let srv = Arc::clone(&self);
            tokio::spawn(async move {
                if let Err(e) = srv.handle_connection(stream).await {
                    eprintln!("Coordinator: connection from {} error: {}", peer, e);
                }
            });
        }
        Ok(())
    }

    // ── Connection handler ────────────────────────────────────────────────────

    async fn handle_connection(&self, mut stream: TcpStream) -> std::io::Result<()> {
        loop {
            // Read request
            let req = match self.read_request(&mut stream).await {
                Ok(r) => r,
                Err(_) => break,
            };

            let resp = match req {
                CoordRequest::Ping => CoordResponse::Pong,
                CoordRequest::Stats => CoordResponse::Stats {
                    connections: 0,
                    total_queries: 0,
                    nodes: 0,
                    edges: 0,
                    shards: self.shard_ids.len(),
                },
                CoordRequest::Disconnect | CoordRequest::Unknown => {
                    let _ = self.write_response(&mut stream, &CoordResponse::Goodbye).await;
                    break;
                }
                CoordRequest::Query { query, .. } => {
                    match self.execute_on_all_shards(&query).await {
                        Ok(rows) => CoordResponse::Result {
                            rows: rows_to_wire(rows),
                        },
                        Err(e) => CoordResponse::Error { message: e },
                    }
                }
            };

            if self.write_response(&mut stream, &resp).await.is_err() {
                break;
            }
        }
        Ok(())
    }

    // ── Query fan-out ─────────────────────────────────────────────────────────

    /// Send `query` to every known shard concurrently and merge results.
    async fn execute_on_all_shards(&self, query: &str) -> Result<Vec<Row>, String> {
        let mut handles = Vec::new();

        for &shard_id in &self.shard_ids {
            if self.pool.contains_key(&shard_id) {
                let q = query.to_string();
                // Clone the Arc so the spawned task owns it.
                let pool = Arc::clone(&self.pool);
                handles.push(tokio::spawn(async move {
                    let mut guard = pool
                        .get(&shard_id)
                        .expect("shard_id must exist in pool")
                        .lock()
                        .await;
                    guard.execute(&q).await.unwrap_or_default()
                }));
            }
        }

        let mut all_rows: Vec<Vec<Row>> = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(rows) => all_rows.push(rows),
                Err(e) => return Err(format!("Task join error: {}", e)),
            }
        }

        Ok(merge_rows(all_rows))
    }

    // ── Wire helpers ──────────────────────────────────────────────────────────

    async fn read_request(&self, stream: &mut TcpStream) -> std::io::Result<CoordRequest> {
        let mut len_buf = [0u8; 4];
        timeout(self.coord_config.timeout, stream.read_exact(&mut len_buf))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "read timeout"))??;

        let msg_len = u32::from_be_bytes(len_buf) as usize;
        let mut buf = vec![0u8; msg_len];
        timeout(self.coord_config.timeout, stream.read_exact(&mut buf))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "read timeout"))??;

        serde_json::from_slice(&buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    async fn write_response(
        &self,
        stream: &mut TcpStream,
        resp: &CoordResponse,
    ) -> std::io::Result<()> {
        let payload = serde_json::to_vec(resp)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let len_bytes = (payload.len() as u32).to_be_bytes();

        timeout(self.coord_config.timeout, async {
            stream.write_all(&len_bytes).await?;
            stream.write_all(&payload).await
        })
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "write timeout"))?
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Deduplicate and merge rows from multiple shards.
fn merge_rows(batches: Vec<Vec<Row>>) -> Vec<Row> {
    use std::collections::HashSet;

    let mut seen: HashSet<Vec<(String, String)>> = HashSet::new();
    let mut out = Vec::new();

    for batch in batches {
        for row in batch {
            let mut key: Vec<(String, String)> = row
                .columns
                .iter()
                .map(|(k, v)| (k.clone(), row_value_to_string(v)))
                .collect();
            key.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
            if seen.insert(key) {
                out.push(row);
            }
        }
    }
    out
}

fn row_value_to_string(v: &RowValue) -> String {
    match v {
        RowValue::Null => "null".to_string(),
        RowValue::Int(n) => n.to_string(),
        RowValue::Text(s) => s.clone(),
        RowValue::Bool(b) => b.to_string(),
    }
}

fn rows_to_wire(rows: Vec<Row>) -> Vec<HashMap<String, String>> {
    rows.into_iter()
        .map(|row| {
            row.columns
                .into_iter()
                .map(|(k, v)| (k, row_value_to_string(&v)))
                .collect()
        })
        .collect()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use maharit_cluster::{ClusterConfig, ShardConfig};

    fn make_config(n_shards: usize) -> (CoordinatorConfig, ClusterConfig) {
        let shards = (0..n_shards as u32)
            .map(|i| ShardConfig {
                id: i,
                address: format!("127.0.0.1:{}", 7000 + i),
            })
            .collect();
        let cc = CoordinatorConfig::default();
        let cluster = ClusterConfig {
            enabled: true,
            strategy: "hash".into(),
            shards,
            replication_factor: 1,
        };
        (cc, cluster)
    }

    #[test]
    fn test_build_shard_pool() {
        let (_, cluster) = make_config(3);
        let pool = build_shard_pool(&cluster);
        assert_eq!(pool.len(), 3);
        assert!(pool.contains_key(&0));
        assert!(pool.contains_key(&2));
    }

    #[test]
    fn test_merge_rows_dedup() {
        let mut r = Row::new();
        r.insert("x", RowValue::Int(1));
        let batches = vec![vec![r.clone()], vec![r]];
        let merged = merge_rows(batches);
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn test_merge_rows_distinct() {
        let mut r1 = Row::new();
        r1.insert("x", RowValue::Int(1));
        let mut r2 = Row::new();
        r2.insert("x", RowValue::Int(2));
        let merged = merge_rows(vec![vec![r1], vec![r2]]);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn test_coordinator_server_new() {
        let (cc, cluster) = make_config(2);
        let srv = ShardCoordinatorServer::new(cc, cluster);
        assert_eq!(srv.shard_ids.len(), 2);
    }

    #[test]
    fn test_rows_to_wire_conversion() {
        let mut r = Row::new();
        r.insert("name", RowValue::Text("Alice".into()));
        r.insert("age", RowValue::Int(30));
        let wire = rows_to_wire(vec![r]);
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0].get("name").unwrap(), "Alice");
        assert_eq!(wire[0].get("age").unwrap(), "30");
    }
}
