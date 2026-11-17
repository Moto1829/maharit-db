//! TCP server for network-based query execution
//!
//! Provides:
//! - TCP connection handling with async I/O
//! - Length-prefixed message framing
//! - Optional TLS (with certificate hot reload)
//! - JSON request/response protocol
//! - Connection pool management
//! - Graceful shutdown

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use bytes::{Buf, BytesMut};
use maharit_core::{ConcurrentGraph, ConstraintManager, FulltextManager, PropertyIndex};
use maharit_query::{AccessPolicy, AstCache, Executor, Parser, Value, is_read_only};
use maharit_storage::TransactionManager;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::time::timeout;
use tracing::Instrument;

use crate::durable_wal::{DurableWal, snapshot_digest};
use crate::mutation_log::RecordingGraph;
use crate::replication::{
    FollowerReplicationManager, LeaderReplicationManager, ReplicaApply, ReplicationConfig,
    ReplicationStats, SnapshotSource, WalEntryData,
};
use crate::schema::Schema;
use crate::tls::{DEFAULT_TLS_RELOAD_INTERVAL, ReloadableTlsAcceptor};
use crate::tracing_setup::TracingConfig;
use maharit_storage::UndoRecord;

mod connection;
mod protocol;
mod query;
mod session;
mod state;

use connection::*;
pub use protocol::*;
use query::*;
use session::*;
pub use state::*;

/// Server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Address to bind to
    pub bind_address: String,
    /// Maximum number of concurrent connections
    pub max_connections: usize,
    /// Read timeout for client connections
    pub read_timeout: Duration,
    /// Write timeout for client connections
    pub write_timeout: Duration,
    /// 認証を必須にするかどうか。デフォルトは `false`（互換性のため）。
    /// `true` のとき、`Login` 以外のリクエストは有効な `sessionToken` を要求する。
    pub require_auth: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:7687".to_string(),
            max_connections: 100,
            read_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(30),
            require_auth: false,
        }
    }
}

/// Default chunk size for streaming results
pub const DEFAULT_CHUNK_SIZE: usize = 100;

/// メッセージ長プレフィックスの上限（バイト）。
/// クライアントが宣言する長さがこれを超える場合は接続を切断する。
/// 悪意あるクライアントが巨大な長さを宣言してサーバーのメモリを枯渇させる
/// DoS を防ぐためのガード。
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024; // 64 MiB

/// セッションタイムアウト（秒）。`AuthManager` の `session_timeout` (30 分) と
/// 揃える。クライアントに返す expires_at の算出に使う。
pub const DEFAULT_SESSION_TIMEOUT_SECS: u64 = 30 * 60;

/// Statistics for the server
#[derive(Debug, Default)]
pub struct ServerStats {
    pub current_connections: AtomicU64,
    pub total_connections: AtomicU64,
    pub total_queries: AtomicU64,
    next_stream_id: AtomicU64,
}

impl ServerStats {
    /// Generate a unique stream ID
    pub fn next_stream_id(&self) -> u64 {
        self.next_stream_id.fetch_add(1, Ordering::SeqCst)
    }
}

/// TCP server for MaharitDB
pub struct TcpServer {
    config: ServerConfig,
    graph: Arc<ConcurrentGraph>,
    stats: Arc<ServerStats>,
    shutdown: Arc<AtomicBool>,
    tx_manager: Arc<TransactionManager>,
    /// Shared constraint / fulltext / property-index managers: persist across
    /// all query executions.
    managers: Arc<SharedManagers>,
    /// Optional leader replication manager: when set, write operations are
    /// automatically replicated to followers via WAL entries.
    replication: SharedLeader,
    /// Optional follower replication manager: when set, the node rejects
    /// writes until it is promoted, and `stats` reports leader liveness.
    follower: Option<Arc<FollowerReplicationManager>>,
    /// How a follower becomes a leader when promoted.
    promotion: Option<Arc<Promotion>>,
    /// Authentication manager. Only enforced when `config.require_auth = true`.
    auth: Arc<Mutex<crate::auth::AuthManager>>,
    /// Shared parsed-AST cache: avoids re-parsing identical query strings on the
    /// hot request path. Keyed by normalized query text.
    ast_cache: Arc<Mutex<AstCache>>,
    /// On-disk WAL + checkpoint settings. `None` keeps the old behaviour of
    /// saving only at shutdown.
    durability: Option<Arc<Durability>>,
    /// TLS for client connections. `None` serves plaintext.
    tls: Option<ReloadableTlsAcceptor>,
    /// Prometheus metrics served by the monitoring HTTP server (`/metrics`).
    metrics: Arc<crate::metrics::Metrics>,
    /// 監査ログ（`--audit-log`）。ログイン・ユーザー管理・権限エラーを記録する。
    audit: Option<Arc<crate::audit::AuditLogger>>,
}

/// Capacity (number of distinct queries) of the shared AST cache.
const AST_CACHE_CAPACITY: usize = 512;

impl TcpServer {
    /// Create a new TCP server
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            graph: Arc::new(ConcurrentGraph::new()),
            stats: Arc::new(ServerStats::default()),
            shutdown: Arc::new(AtomicBool::new(false)),
            tx_manager: Arc::new(TransactionManager::new()),
            managers: Arc::new(SharedManagers::new()),
            replication: SharedLeader::default(),
            follower: None,
            promotion: None,
            auth: Arc::new(Mutex::new(crate::auth::AuthManager::new())),
            ast_cache: Arc::new(Mutex::new(AstCache::new(AST_CACHE_CAPACITY))),
            durability: None,
            tls: None,
            metrics: Arc::new(crate::metrics::Metrics::new()),
            audit: None,
        }
    }

    /// Create a server with an existing graph
    pub fn with_graph(config: ServerConfig, graph: ConcurrentGraph) -> Self {
        Self {
            config,
            graph: Arc::new(graph),
            stats: Arc::new(ServerStats::default()),
            shutdown: Arc::new(AtomicBool::new(false)),
            tx_manager: Arc::new(TransactionManager::new()),
            managers: Arc::new(SharedManagers::new()),
            replication: SharedLeader::default(),
            follower: None,
            promotion: None,
            auth: Arc::new(Mutex::new(crate::auth::AuthManager::new())),
            ast_cache: Arc::new(Mutex::new(AstCache::new(AST_CACHE_CAPACITY))),
            durability: None,
            tls: None,
            metrics: Arc::new(crate::metrics::Metrics::new()),
            audit: None,
        }
    }

    /// Create a server with a shared graph Arc (for sharing with signal handlers)
    pub fn with_graph_arc(config: ServerConfig, graph: Arc<ConcurrentGraph>) -> Self {
        Self {
            config,
            graph,
            stats: Arc::new(ServerStats::default()),
            shutdown: Arc::new(AtomicBool::new(false)),
            tx_manager: Arc::new(TransactionManager::new()),
            managers: Arc::new(SharedManagers::new()),
            replication: SharedLeader::default(),
            follower: None,
            promotion: None,
            auth: Arc::new(Mutex::new(crate::auth::AuthManager::new())),
            ast_cache: Arc::new(Mutex::new(AstCache::new(AST_CACHE_CAPACITY))),
            durability: None,
            tls: None,
            metrics: Arc::new(crate::metrics::Metrics::new()),
            audit: None,
        }
    }

    /// The metrics this server records (queries, errors, latency, connections,
    /// graph size). Hand it to the monitoring HTTP server.
    pub fn metrics(&self) -> Arc<crate::metrics::Metrics> {
        self.metrics.set_node_count(self.graph.node_count() as i64);
        self.metrics.set_edge_count(self.graph.edge_count() as i64);
        Arc::clone(&self.metrics)
    }

    /// Return a clone of the graph Arc
    pub fn graph_arc(&self) -> Arc<ConcurrentGraph> {
        Arc::clone(&self.graph)
    }

    /// Replace the authentication manager (e.g. to set a non-default admin
    /// password before enabling `require_auth`).
    pub fn with_auth(mut self, auth: crate::auth::AuthManager) -> Self {
        self.auth = Arc::new(Mutex::new(auth));
        self
    }

    /// 監査ログを有効にする（ログイン成功・失敗、ユーザー管理、権限エラーを記録）。
    pub fn with_audit_log(mut self, logger: crate::audit::AuditLogger) -> Self {
        self.audit = Some(Arc::new(logger));
        self
    }

    /// Attach a leader replication manager.  Once attached, every successful
    /// write query automatically appends WAL entries to the manager, which
    /// broadcasts them to all connected followers.
    pub fn with_replication(self, manager: Arc<LeaderReplicationManager>) -> Self {
        manager.set_snapshot_source(snapshot_source(&self.graph, &self.managers));
        *self.replication.write().unwrap_or_else(|e| e.into_inner()) = Some(manager);
        self
    }

    /// Let a follower be promoted with the `promote` request: it then stops
    /// following, accepts writes and serves replication as `promotion` says.
    pub fn with_promotion(mut self, promotion: Promotion) -> Self {
        self.promotion = Some(Arc::new(promotion));
        self
    }

    /// Log every write to an on-disk WAL before replying, and checkpoint it
    /// into the snapshot. Do not use on a follower: replicated changes bypass
    /// the WAL.
    pub fn with_durability(mut self, durability: Durability) -> Self {
        self.durability = Some(Arc::new(durability));
        self
    }

    /// A handle that checkpoints this server's snapshot + WAL (e.g. on
    /// shutdown). `None` when no WAL is configured.
    pub fn checkpointer(&self) -> Option<Checkpointer> {
        self.durability.as_ref().map(|d| Checkpointer {
            graph: Arc::clone(&self.graph),
            managers: Arc::clone(&self.managers),
            durability: Arc::clone(d),
        })
    }

    /// Serve client connections over TLS. Connections that do not complete a
    /// TLS handshake within `read_timeout` are closed.
    pub fn with_tls(mut self, tls: ReloadableTlsAcceptor) -> Self {
        self.tls = Some(tls);
        self
    }

    /// Attach a follower replication manager so the `stats` response can report
    /// this follower's view of leader liveness.
    pub fn with_follower(mut self, manager: Arc<FollowerReplicationManager>) -> Self {
        manager.set_apply_hook(Arc::clone(&self.managers) as Arc<dyn ReplicaApply>);
        self.follower = Some(manager);
        self
    }

    /// Install `schema` (constraints and indexes, e.g. loaded from the data
    /// file and WAL) and build its indexes from the current graph.
    pub fn with_schema(self, schema: &Schema) -> Result<Self, String> {
        self.managers.install_schema(&self.graph, schema)?;
        Ok(self)
    }

    /// Saves the graph and schema to a data file (for shutdown without a WAL).
    pub fn snapshot_writer(&self) -> SnapshotWriter {
        SnapshotWriter {
            graph: Arc::clone(&self.graph),
            managers: Arc::clone(&self.managers),
        }
    }

    /// Start the server
    pub async fn start(&self) -> std::io::Result<()> {
        // Initialise structured tracing (JSON to stderr; honours RUST_LOG env var)
        let _tracing_guard = TracingConfig::default().init();

        let listener = TcpListener::bind(&self.config.bind_address).await?;
        let transport = if self.tls.is_some() {
            "TLS"
        } else {
            "plaintext"
        };
        tracing::info!(address = %self.config.bind_address, transport, "Server listening");
        println!(
            "Server listening on {} ({})",
            self.config.bind_address, transport
        );
        if self.tls.is_none() && !is_loopback_bind(&self.config.bind_address) {
            tracing::warn!(
                "client connections are NOT encrypted; use --tls-cert/--tls-key when the port is reachable from other hosts"
            );
            eprintln!(
                "WARN: client connections are NOT encrypted (no --tls-cert/--tls-key) and the server listens on {}.",
                self.config.bind_address
            );
        }

        // 認証無効時に警告を出す（運用者が見落とさないよう WARN レベル）
        if !self.config.require_auth {
            tracing::warn!(
                "maharit-server authentication is DISABLED. All requests are accepted without sessionToken. Set ServerConfig::require_auth=true to enforce login."
            );
            eprintln!(
                "WARN: maharit-server authentication is DISABLED. All requests are accepted without sessionToken."
            );
        }

        self.start_with_listener(listener).await
    }

    /// Start the server with an existing listener.
    ///
    /// Useful for testing: bind to port 0 externally, retrieve the actual
    /// address via `listener.local_addr()`, then pass the listener here.
    pub async fn start_with_listener(&self, listener: TcpListener) -> std::io::Result<()> {
        // Create shutdown broadcast channel
        let (shutdown_tx, _) = broadcast::channel::<()>(1);

        if let Some(tls) = &self.tls {
            tls.spawn_watcher(DEFAULT_TLS_RELOAD_INTERVAL, Arc::clone(&self.shutdown));
        }

        // Checkpoint whenever the WAL grows past the configured size.
        if let Some(checkpointer) = self.checkpointer()
            && checkpointer.durability.checkpoint_bytes > 0
        {
            let shutdown = Arc::clone(&self.shutdown);
            tokio::spawn(async move {
                let limit = checkpointer.durability.checkpoint_bytes;
                let mut ticker = tokio::time::interval(Duration::from_secs(1));
                while !shutdown.load(Ordering::SeqCst) {
                    ticker.tick().await;
                    if checkpointer.durability.wal.len() < limit {
                        continue;
                    }
                    match checkpointer.checkpoint().await {
                        Ok(()) => tracing::info!("checkpoint completed"),
                        Err(e) => tracing::error!(error = %e, "checkpoint failed"),
                    }
                }
            });
        }

        loop {
            if self.shutdown.load(Ordering::SeqCst) {
                println!("Server shutting down...");
                break;
            }

            // Check connection limit
            let current = self.stats.current_connections.load(Ordering::SeqCst);
            if current >= self.config.max_connections as u64 {
                // Wait a bit before accepting more connections
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }

            // Accept with timeout to allow checking shutdown flag
            let accept_result = timeout(Duration::from_secs(1), listener.accept()).await;

            match accept_result {
                Ok(Ok((socket, addr))) => {
                    tracing::info!(peer = %addr, "Client connected");
                    self.stats.total_connections.fetch_add(1, Ordering::SeqCst);
                    self.stats
                        .current_connections
                        .fetch_add(1, Ordering::SeqCst);
                    self.metrics.increment_connections();
                    let metrics = Arc::clone(&self.metrics);

                    let graph = Arc::clone(&self.graph);
                    let stats = Arc::clone(&self.stats);
                    let shutdown = Arc::clone(&self.shutdown);
                    let tx_manager = Arc::clone(&self.tx_manager);
                    let managers = Arc::clone(&self.managers);
                    let config = self.config.clone();
                    let replication = Arc::clone(&self.replication);
                    let follower = self.follower.clone();
                    let promotion = self.promotion.clone();
                    let auth = Arc::clone(&self.auth);
                    let ast_cache = Arc::clone(&self.ast_cache);
                    let durability = self.durability.clone();
                    let audit = self.audit.clone();
                    let tls = self.tls.as_ref().map(ReloadableTlsAcceptor::acceptor);
                    let mut shutdown_rx = shutdown_tx.subscribe();

                    tokio::spawn(async move {
                        let ctx = ConnectionContext {
                            graph,
                            stats: stats.clone(),
                            shutdown,
                            tx_manager,
                            managers,
                            config,
                            replication,
                            follower,
                            promotion,
                            auth,
                            ast_cache,
                            durability,
                            metrics: Arc::clone(&metrics),
                            audit,
                        };
                        let result = match tls {
                            None => handle_connection(socket, ctx, &mut shutdown_rx).await,
                            // The handshake runs in the connection task so a slow
                            // client cannot stall the accept loop.
                            Some(acceptor) => {
                                match timeout(ctx.config.read_timeout, acceptor.accept(socket))
                                    .await
                                {
                                    Ok(Ok(tls_stream)) => {
                                        handle_connection(tls_stream, ctx, &mut shutdown_rx).await
                                    }
                                    Ok(Err(e)) => Err(std::io::Error::new(
                                        e.kind(),
                                        format!("TLS handshake failed: {e}"),
                                    )),
                                    Err(_) => Err(std::io::Error::new(
                                        std::io::ErrorKind::TimedOut,
                                        "TLS handshake timed out",
                                    )),
                                }
                            }
                        };

                        if let Err(e) = result {
                            tracing::warn!(peer = %addr, error = %e, "Connection error");
                            eprintln!("Connection error from {}: {}", addr, e);
                        } else {
                            tracing::info!(peer = %addr, "Client disconnected");
                        }

                        stats.current_connections.fetch_sub(1, Ordering::SeqCst);
                        metrics.decrement_connections();
                    });
                }
                Ok(Err(e)) => {
                    eprintln!("Accept error: {}", e);
                }
                Err(_) => {
                    // Timeout, just continue to check shutdown flag
                }
            }
        }

        Ok(())
    }

    /// Request graceful shutdown
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Get current statistics
    pub fn stats(&self) -> &ServerStats {
        &self.stats
    }
}

/// Whether `bind_address` only accepts connections from this host.
pub(crate) fn is_loopback_bind(bind_address: &str) -> bool {
    let host = bind_address
        .rsplit_once(':')
        .map_or(bind_address, |(h, _)| h)
        .trim_start_matches('[')
        .trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
#[allow(clippy::approx_constant)] // 3.14 等は PI の近似ではなくリテラルのテストデータ
mod tests;
