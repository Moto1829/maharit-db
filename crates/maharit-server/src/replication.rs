//! Leader/follower replication for MaharitDB
//!
//! Provides:
//! - WAL-based replication between leader and follower nodes
//! - Heartbeat mechanism for detecting leader failure
//! - Length-prefixed JSON message framing (same as tcp_server.rs)
//! - Async TCP communication using tokio, optionally over TLS

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use maharit_core::{ConcurrentGraph, Graph};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex as AsyncMutex, RwLock, mpsc};
use tokio::time::{interval, timeout};

use crate::mutation_log::{property_value_from_wal_string, property_value_to_wal_string};
use crate::tls::ReloadableTlsAcceptor;

/// How long a follower waits before reconnecting to the leader.
const FOLLOWER_RECONNECT_DELAY: Duration = Duration::from_secs(1);

/// How long a TLS handshake on the replication channel may take.
const REPLICATION_TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// TLS settings a follower uses to connect to (and verify) the leader.
#[derive(Clone)]
pub struct LeaderTls {
    /// Client configuration holding the CA certificates to trust.
    pub client_config: Arc<rustls::ClientConfig>,
    /// Name the leader's certificate must be valid for (DNS name or IP).
    pub server_name: rustls::pki_types::ServerName<'static>,
}

/// Error types for replication operations
#[derive(Debug, thiserror::Error)]
pub enum ReplicationError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Connection to leader failed: {0}")]
    ConnectionFailed(String),
    #[error("Leader not configured")]
    LeaderNotConfigured,
    #[error("Replication timeout")]
    Timeout,
}

/// Node role in the replication cluster
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeRole {
    /// This node accepts writes and replicates to followers
    Leader,
    /// This node receives replicated writes from the leader
    Follower,
}

/// Configuration for the replication subsystem
#[derive(Debug, Clone)]
pub struct ReplicationConfig {
    /// The role of this node in the cluster
    pub role: NodeRole,
    /// Unique identifier for this node
    pub node_id: String,
    /// Address to listen on for incoming replication connections (leader only)
    pub replication_bind_address: String,
    /// Address of the leader to connect to (follower only)
    pub leader_address: Option<String>,
    /// How often the leader sends heartbeats, in seconds
    pub heartbeat_interval_secs: u64,
    /// How long before a follower considers the leader dead, in seconds
    pub heartbeat_timeout_secs: u64,
    /// Shared secret authenticating the replication channel.
    ///
    /// When `Some`, the leader rejects any follower whose handshake does not
    /// present the same secret, and followers include it in their handshake.
    /// When `None`, the channel is unauthenticated (any peer that can reach
    /// the port may replicate) — only safe on a fully trusted network.
    pub shared_secret: Option<String>,
}

impl Default for ReplicationConfig {
    fn default() -> Self {
        Self {
            role: NodeRole::Leader,
            node_id: "node-1".to_string(),
            replication_bind_address: "127.0.0.1:7688".to_string(),
            leader_address: None,
            heartbeat_interval_secs: 1,
            heartbeat_timeout_secs: 5,
            shared_secret: None,
        }
    }
}

/// WAL (Write-Ahead Log) entry payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WalEntryData {
    /// A node was created (supports multiple labels)
    CreateNode { node_id: u64, labels: Vec<String> },
    /// A node was deleted
    DeleteNode { node_id: u64 },
    /// An edge was created
    CreateEdge {
        edge_id: u64,
        from: u64,
        to: u64,
        label: String,
    },
    /// An edge was deleted
    DeleteEdge { edge_id: u64 },
    /// A property was set on a node or edge
    SetProperty {
        target_id: u64,
        is_node: bool,
        key: String,
        value: String,
    },
    /// A property was removed from a node or edge
    RemoveProperty {
        target_id: u64,
        is_node: bool,
        key: String,
    },
    /// A label was added to a node
    AddLabel { node_id: u64, label: String },
    /// A label was removed from a node
    RemoveLabel { node_id: u64, label: String },
    /// The complete schema after a DDL statement (replaces the previous one).
    SetSchema { schema: crate::schema::Schema },
}

/// Messages exchanged between leader and follower over the replication channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReplicationMessage {
    /// Follower -> Leader: initiate a replication session.
    /// `current_lsn` is the follower's current applied LSN (0 for a new follower).
    Handshake {
        follower_id: String,
        #[serde(default)]
        current_lsn: u64,
        /// Shared secret proving the follower is authorized to replicate.
        /// `None` when the channel is configured without authentication.
        #[serde(default)]
        auth_token: Option<String>,
    },
    /// Leader -> Follower: acknowledge the handshake
    HandshakeAck { leader_id: String },
    /// Leader -> Follower: the handshake was rejected (e.g. bad shared secret).
    Unauthorized { reason: String },
    /// Leader -> Follower: periodic liveness signal
    Heartbeat {
        leader_id: String,
        timestamp: u64,
        lsn: u64,
    },
    /// Follower -> Leader: acknowledge a heartbeat
    HeartbeatAck { follower_id: String, lsn: u64 },
    /// Leader -> Follower: replicate a WAL entry
    WalEntry { lsn: u64, entry: WalEntryData },
    /// Follower -> Leader: confirm WAL entry was applied
    WalAck { follower_id: String, lsn: u64 },
    /// Leader -> Follower: full graph snapshot, sent right after the handshake.
    /// `data` is a JSON-serialized `Vec<WalEntryData>` that recreates the graph;
    /// the follower replaces its graph with it. `lsn` is the leader's LSN at the
    /// moment of the snapshot: every later change arrives as a `WalEntry`.
    Snapshot {
        data: Vec<u8>,
        #[serde(default)]
        lsn: u64,
    },
    /// Obsolete (sent by older `admin promote-to-leader`); followers ignore it.
    /// Promotion is the `promote` request on the client port.
    PromoteToLeader,
    /// Leader -> Follower: leader is shutting down
    Shutdown,
}

/// Tracks the state of a connected follower on the leader side
#[derive(Debug, Clone)]
pub struct FollowerState {
    /// Unique identifier of the follower node
    pub follower_id: String,
    /// Last log sequence number the follower acknowledged
    pub last_lsn: u64,
    /// When the follower last sent any message
    pub last_heartbeat: Instant,
    /// Whether the follower connection is considered healthy
    pub is_alive: bool,
}

/// Replication statistics snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationStats {
    /// "Leader" or "Follower"
    pub role: String,
    /// This node's ID
    pub node_id: String,
    /// Current log sequence number
    pub current_lsn: u64,
    /// Number of connected followers (leader only; 0 for follower)
    pub follower_count: usize,
    /// Whether the leader is reachable (follower only; true for leader)
    pub is_leader_alive: bool,
}

// ─── Message framing helpers ────────────────────────────────────────────────

/// Write a length-prefixed JSON message to an async writer
async fn send_message<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    msg: &ReplicationMessage,
) -> Result<(), ReplicationError> {
    let data = serde_json::to_vec(msg)?;
    let len = data.len() as u32;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&data).await?;
    writer.flush().await?;
    Ok(())
}

/// Upper bound on a single replication message (bytes).  Guards against a
/// peer declaring a huge length prefix to force a multi-gigabyte allocation.
///
/// Set generously because a new follower receives the entire graph as one
/// `Snapshot` message; the cap only blocks the extreme (up to ~4 GiB) values
/// an attacker could put in the u32 length prefix.
const MAX_REPLICATION_MESSAGE_SIZE: usize = 1024 * 1024 * 1024; // 1 GiB

/// Per-follower WAL queue capacity. When full, `append_wal_entry` waits
/// (backpressure) instead of dropping entries.
const WAL_QUEUE_CAPACITY: usize = 1024;

/// How long `append_wal_entry` waits for a follower with a full queue before
/// disconnecting it.
pub const WAL_SEND_TIMEOUT: Duration = Duration::from_secs(5);

type WalSubscribers = Arc<AsyncMutex<Vec<mpsc::Sender<(u64, WalEntryData)>>>>;

/// Read a length-prefixed JSON message from an async reader
async fn recv_message<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> Result<ReplicationMessage, ReplicationError> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_REPLICATION_MESSAGE_SIZE {
        return Err(ReplicationError::ConnectionFailed(format!(
            "replication message length {} exceeds maximum {}",
            len, MAX_REPLICATION_MESSAGE_SIZE
        )));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

/// Return the current Unix timestamp in seconds
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ─── Leader ─────────────────────────────────────────────────────────────────

/// Manages outbound replication on the leader node.
///
/// Accepts follower connections on a dedicated TCP port, streams WAL entries
/// to all connected followers, and sends periodic heartbeats.
pub struct LeaderReplicationManager {
    config: ReplicationConfig,
    /// Monotonically increasing log sequence number
    lsn: Arc<AtomicU64>,
    /// Map of follower_id -> state for every connected follower
    followers: Arc<RwLock<HashMap<String, FollowerState>>>,
    /// Set to `true` to request an orderly shutdown
    shutdown: Arc<AtomicBool>,
    /// Per-follower WAL queues (one bounded channel per authenticated
    /// follower handler).
    wal_subscribers: WalSubscribers,
    /// Optional graph reference for sending full snapshots to new followers.
    graph: Option<Arc<RwLock<Graph>>>,
    /// When set, followers must connect over TLS.
    tls: Option<ReloadableTlsAcceptor>,
    /// When set, every follower receives a consistent snapshot of the
    /// server's graph on connect (see [`SnapshotSource`]).
    snapshot_source: Arc<std::sync::OnceLock<SnapshotSource>>,
}

impl LeaderReplicationManager {
    /// Create a new leader replication manager with the given configuration.
    ///
    /// # Panics
    /// Does not panic; errors are surfaced through `start()`.
    pub fn new(config: ReplicationConfig) -> Self {
        Self {
            config,
            lsn: Arc::new(AtomicU64::new(0)),
            followers: Arc::new(RwLock::new(HashMap::new())),
            shutdown: Arc::new(AtomicBool::new(false)),
            wal_subscribers: Arc::new(AsyncMutex::new(Vec::new())),
            graph: None,
            tls: None,
            snapshot_source: Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Create a leader manager with a graph reference for initial snapshot sync.
    pub fn with_graph(config: ReplicationConfig, graph: Arc<RwLock<Graph>>) -> Self {
        Self {
            config,
            lsn: Arc::new(AtomicU64::new(0)),
            followers: Arc::new(RwLock::new(HashMap::new())),
            shutdown: Arc::new(AtomicBool::new(false)),
            wal_subscribers: Arc::new(AsyncMutex::new(Vec::new())),
            graph: Some(graph),
            tls: None,
            snapshot_source: Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Send followers a snapshot of `source` when they connect. Only the first
    /// call has an effect.
    pub fn set_snapshot_source(&self, source: SnapshotSource) {
        let _ = self.snapshot_source.set(source);
    }

    /// Accept follower connections over TLS only. This also keeps the shared
    /// secret from crossing the network in plaintext.
    pub fn with_tls(mut self, tls: ReloadableTlsAcceptor) -> Self {
        self.tls = Some(tls);
        self
    }

    /// Append a WAL entry and stream it to every connected follower.
    ///
    /// Increments and returns the new log sequence number. Delivery applies
    /// backpressure: if a follower's queue is full this waits for it to drain,
    /// so bursts larger than the queue (e.g. one statement touching thousands
    /// of elements) are never dropped. A follower that does not drain within
    /// [`WAL_SEND_TIMEOUT`] is disconnected (and logged) rather than silently
    /// left behind. Entries are delivered in LSN order.
    pub async fn append_wal_entry(&self, entry: WalEntryData) -> u64 {
        // Holding the subscriber lock across assignment and delivery keeps
        // LSN order identical to delivery order for concurrent callers.
        let mut subscribers = self.wal_subscribers.lock().await;
        let new_lsn = self.lsn.fetch_add(1, Ordering::SeqCst) + 1;
        let mut alive = Vec::with_capacity(subscribers.len());
        for tx in subscribers.drain(..) {
            match timeout(WAL_SEND_TIMEOUT, tx.send((new_lsn, entry.clone()))).await {
                Ok(Ok(())) => alive.push(tx),
                // Handler already gone (follower disconnected).
                Ok(Err(_)) => {}
                Err(_) => {
                    tracing::error!(
                        lsn = new_lsn,
                        "replication: follower did not drain its WAL queue within {:?}, dropping it",
                        WAL_SEND_TIMEOUT
                    );
                    eprintln!(
                        "Replication: follower too slow (WAL queue full for {:?}); disconnecting",
                        WAL_SEND_TIMEOUT
                    );
                }
            }
        }
        *subscribers = alive;
        new_lsn
    }

    /// Return the number of followers currently tracked as alive.
    pub fn get_follower_count(&self) -> usize {
        // Try a non-blocking read; fall back to 0 if the lock is contended.
        match self.followers.try_read() {
            Ok(guard) => guard.values().filter(|f| f.is_alive).count(),
            Err(_) => 0,
        }
    }

    /// Return a statistics snapshot for this node.
    pub fn get_stats(&self) -> ReplicationStats {
        ReplicationStats {
            role: "Leader".to_string(),
            node_id: self.config.node_id.clone(),
            current_lsn: self.lsn.load(Ordering::SeqCst),
            follower_count: self.get_follower_count(),
            is_leader_alive: true,
        }
    }

    /// Signal all background tasks to stop.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Start the replication listener.
    ///
    /// Spawns a background tokio task that accepts follower connections and
    /// handles each one in its own task.  Returns `Ok(())` immediately after
    /// the listener is bound.
    pub async fn start(&self) -> Result<(), ReplicationError> {
        let listener = TcpListener::bind(&self.config.replication_bind_address).await?;
        self.start_with_listener(listener).await
    }

    /// Start the replication listener with an existing `TcpListener`.
    ///
    /// Useful for tests: bind to port 0 externally, retrieve the actual address
    /// via `listener.local_addr()`, then pass the listener here.
    pub async fn start_with_listener(&self, listener: TcpListener) -> Result<(), ReplicationError> {
        let config = self.config.clone();
        let lsn = Arc::clone(&self.lsn);
        let followers = Arc::clone(&self.followers);
        let shutdown = Arc::clone(&self.shutdown);
        let wal_subscribers = Arc::clone(&self.wal_subscribers);
        let graph = self.graph.clone();
        let tls = self.tls.clone();
        // Read per connection: the source may be set after the listener starts.
        let snapshot_source = Arc::clone(&self.snapshot_source);

        tokio::spawn(async move {
            loop {
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }

                let accept_result = timeout(Duration::from_secs(1), listener.accept()).await;

                match accept_result {
                    Ok(Ok((socket, _addr))) => {
                        let config2 = config.clone();
                        let lsn2 = Arc::clone(&lsn);
                        let followers2 = Arc::clone(&followers);
                        let shutdown2 = Arc::clone(&shutdown);
                        let wal_subscribers2 = Arc::clone(&wal_subscribers);
                        let graph2 = graph.clone();
                        let acceptor = tls.as_ref().map(ReloadableTlsAcceptor::acceptor);
                        let snapshot_source2 = snapshot_source.get().cloned();

                        tokio::spawn(async move {
                            let ctx = LeaderConnection {
                                config: config2,
                                lsn: lsn2,
                                followers: followers2,
                                shutdown: shutdown2,
                                wal_subscribers: wal_subscribers2,
                                graph: graph2,
                                snapshot_source: snapshot_source2,
                            };
                            let result = match acceptor {
                                None => handle_follower_connection(socket, ctx).await,
                                Some(acceptor) => match timeout(
                                    REPLICATION_TLS_HANDSHAKE_TIMEOUT,
                                    acceptor.accept(socket),
                                )
                                .await
                                {
                                    Ok(Ok(stream)) => handle_follower_connection(stream, ctx).await,
                                    Ok(Err(e)) => Err(ReplicationError::ConnectionFailed(format!(
                                        "TLS handshake failed: {e}"
                                    ))),
                                    Err(_) => Err(ReplicationError::ConnectionFailed(
                                        "TLS handshake timed out".to_string(),
                                    )),
                                },
                            };
                            if let Err(e) = result {
                                eprintln!("Follower connection error: {}", e);
                            }
                        });
                    }
                    Ok(Err(e)) => {
                        eprintln!("Replication accept error: {}", e);
                    }
                    Err(_) => {
                        // Timeout — loop back and check shutdown flag.
                    }
                }
            }
        });

        Ok(())
    }
}

/// Constant-time comparison of two replication secrets to avoid leaking the
/// secret through response timing.
fn replication_secret_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Serialize a concurrent graph as WAL entries that recreate it (IDs kept).
pub(crate) fn concurrent_graph_snapshot_entries(g: &ConcurrentGraph) -> Vec<WalEntryData> {
    let mut entries = Vec::new();
    for node in g.nodes() {
        entries.push(WalEntryData::CreateNode {
            node_id: node.id,
            labels: node.labels.clone(),
        });
        for (key, val) in node.properties.iter() {
            entries.push(WalEntryData::SetProperty {
                target_id: node.id,
                is_node: true,
                key: key.clone(),
                value: property_value_to_wal_string(val),
            });
        }
    }
    for edge in g.edges() {
        entries.push(WalEntryData::CreateEdge {
            edge_id: edge.id,
            from: edge.from,
            to: edge.to,
            label: edge.label.clone(),
        });
        for (key, val) in edge.properties.iter() {
            entries.push(WalEntryData::SetProperty {
                target_id: edge.id,
                is_node: false,
                key: key.clone(),
                value: property_value_to_wal_string(val),
            });
        }
    }
    entries
}

/// Where a leader takes the snapshot for a newly connected follower: the
/// server's graph, read while holding the server's write gate so that the
/// snapshot and the WAL stream that follows it neither overlap nor leave a gap.
#[derive(Clone)]
pub struct SnapshotSource {
    pub graph: Arc<ConcurrentGraph>,
    pub write_gate: Arc<AsyncMutex<()>>,
    /// The current schema, sent after the graph so followers build indexes.
    pub schema: Arc<dyn Fn() -> crate::schema::Schema + Send + Sync>,
}

/// Serialize the current graph as a list of WAL entries suitable for snapshot sync.
async fn graph_to_snapshot_entries(graph: &Arc<RwLock<Graph>>) -> Vec<WalEntryData> {
    let g = graph.read().await;
    let mut entries = Vec::new();

    for node in g.nodes() {
        entries.push(WalEntryData::CreateNode {
            node_id: node.id,
            labels: node.labels.clone(),
        });
        for (key, val) in node.properties.iter() {
            let value = property_value_to_wal_string(val);
            entries.push(WalEntryData::SetProperty {
                target_id: node.id,
                is_node: true,
                key: key.clone(),
                value,
            });
        }
    }

    for edge in g.edges() {
        entries.push(WalEntryData::CreateEdge {
            edge_id: edge.id,
            from: edge.from,
            to: edge.to,
            label: edge.label.clone(),
        });
        for (key, val) in edge.properties.iter() {
            let value = property_value_to_wal_string(val);
            entries.push(WalEntryData::SetProperty {
                target_id: edge.id,
                is_node: false,
                key: key.clone(),
                value,
            });
        }
    }

    entries
}

/// Leader state shared with one follower connection task.
struct LeaderConnection {
    config: ReplicationConfig,
    lsn: Arc<AtomicU64>,
    followers: Arc<RwLock<HashMap<String, FollowerState>>>,
    shutdown: Arc<AtomicBool>,
    wal_subscribers: WalSubscribers,
    graph: Option<Arc<RwLock<Graph>>>,
    snapshot_source: Option<SnapshotSource>,
}

/// Handle a single follower connection (plain TCP or TLS) on the leader side.
async fn handle_follower_connection<S>(
    socket: S,
    ctx: LeaderConnection,
) -> Result<(), ReplicationError>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let LeaderConnection {
        config,
        lsn,
        followers,
        shutdown,
        wal_subscribers,
        graph,
        snapshot_source,
    } = ctx;
    let (mut reader, mut writer) = tokio::io::split(socket);

    // ── Handshake ────────────────────────────────────────────────────────────
    let msg = recv_message(&mut reader).await?;
    let (follower_id, follower_lsn, auth_token) = match msg {
        ReplicationMessage::Handshake {
            follower_id,
            current_lsn,
            auth_token,
        } => (follower_id, current_lsn, auth_token),
        other => {
            return Err(ReplicationError::ConnectionFailed(format!(
                "Expected Handshake, got {:?}",
                other
            )));
        }
    };

    // ── Authenticate the follower ─────────────────────────────────────────────
    // When a shared secret is configured, reject any follower that does not
    // present the matching token before disclosing any graph data.  Without
    // this, any peer reaching the replication port could receive a full
    // snapshot and the ongoing WAL stream.
    if let Some(ref expected) = config.shared_secret {
        let ok = auth_token
            .as_deref()
            .map(|t| replication_secret_eq(t, expected))
            .unwrap_or(false);
        if !ok {
            let _ = send_message(
                &mut writer,
                &ReplicationMessage::Unauthorized {
                    reason: "invalid replication secret".to_string(),
                },
            )
            .await;
            return Err(ReplicationError::ConnectionFailed(format!(
                "follower {} presented an invalid replication secret",
                follower_id
            )));
        }
    }

    send_message(
        &mut writer,
        &ReplicationMessage::HandshakeAck {
            leader_id: config.node_id.clone(),
        },
    )
    .await?;

    // Subscribe to the WAL only after the follower authenticated, so an
    // unauthenticated peer can never exert backpressure on the leader.
    let (wal_tx, mut wal_rx) = mpsc::channel(WAL_QUEUE_CAPACITY);

    if let Some(source) = snapshot_source {
        // Writers publish to followers while holding the write gate, so under
        // the gate the graph matches the WAL up to the current LSN exactly:
        // the snapshot covers everything before it and the subscription
        // everything after it.
        let (entries, snapshot_lsn) = {
            let _gate = source.write_gate.lock().await;
            wal_subscribers.lock().await.push(wal_tx);
            let mut entries = concurrent_graph_snapshot_entries(&source.graph);
            entries.push(WalEntryData::SetSchema {
                schema: (source.schema)(),
            });
            (entries, lsn.load(Ordering::SeqCst))
        };
        let data = serde_json::to_vec(&entries)?;
        send_message(
            &mut writer,
            &ReplicationMessage::Snapshot {
                data,
                lsn: snapshot_lsn,
            },
        )
        .await?;
    } else {
        wal_subscribers.lock().await.push(wal_tx);
        // If the follower has no data (LSN == 0), send a full snapshot.
        if follower_lsn == 0
            && let Some(ref g) = graph
        {
            let entries = graph_to_snapshot_entries(g).await;
            let data = serde_json::to_vec(&entries).unwrap_or_default();
            let lsn = lsn.load(Ordering::SeqCst);
            send_message(&mut writer, &ReplicationMessage::Snapshot { data, lsn }).await?;
        }
    }

    // Register the follower.
    {
        let mut guard = followers.write().await;
        guard.insert(
            follower_id.clone(),
            FollowerState {
                follower_id: follower_id.clone(),
                last_lsn: follower_lsn,
                last_heartbeat: Instant::now(),
                is_alive: true,
            },
        );
    }

    let heartbeat_interval = Duration::from_secs(config.heartbeat_interval_secs);
    let mut hb_ticker = interval(heartbeat_interval);

    // Read follower messages on a dedicated task. `recv_message` is not
    // cancellation-safe (it may have consumed part of a frame when dropped), so
    // it must never be polled directly as a `select!` branch: when a WAL send or
    // heartbeat won the race mid-read, the partial frame was lost and the
    // stream desynchronised (task120). `mpsc::Receiver::recv` is cancel-safe.
    let (msg_tx, mut msg_rx) = tokio::sync::mpsc::channel(64);
    let reader_task = tokio::spawn(async move {
        loop {
            let result = recv_message(&mut reader).await;
            let failed = result.is_err();
            if msg_tx.send(result).await.is_err() || failed {
                break;
            }
        }
    });

    // ── Main loop ────────────────────────────────────────────────────────────
    loop {
        if shutdown.load(Ordering::SeqCst) {
            let _ = send_message(&mut writer, &ReplicationMessage::Shutdown).await;
            break;
        }

        tokio::select! {
            // Periodic heartbeat tick
            _ = hb_ticker.tick() => {
                let hb = ReplicationMessage::Heartbeat {
                    leader_id: config.node_id.clone(),
                    timestamp: current_timestamp(),
                    lsn: lsn.load(Ordering::SeqCst),
                };
                if let Err(e) = send_message(&mut writer, &hb).await {
                    tracing::warn!(follower = %follower_id, error = %e, "replication: heartbeat send failed, dropping follower");
                    break;
                }
            }

            // New WAL entry to forward
            entry_result = wal_rx.recv() => {
                match entry_result {
                    Some((entry_lsn, entry_data)) => {
                        let wal_msg = ReplicationMessage::WalEntry {
                            lsn: entry_lsn,
                            entry: entry_data,
                        };
                        if let Err(e) = send_message(&mut writer, &wal_msg).await {
                            tracing::warn!(follower = %follower_id, error = %e, "replication: WAL send failed, dropping follower");
                            break;
                        }
                    }
                    None => {
                        // The leader dropped this follower's queue (too slow).
                        tracing::warn!(follower = %follower_id, "replication: WAL queue closed by leader, dropping follower");
                        break;
                    }
                }
            }

            // Incoming message from the follower (via the reader task)
            read_result = msg_rx.recv() => {
                match read_result {
                    None => {
                        tracing::warn!(follower = %follower_id, "replication: follower reader task ended");
                        break;
                    }
                    Some(read_result) => match read_result {
                    Ok(ReplicationMessage::HeartbeatAck { follower_id: fid, lsn: ack_lsn }) => {
                        let mut guard = followers.write().await;
                        if let Some(state) = guard.get_mut(&fid) {
                            state.last_lsn = ack_lsn;
                            state.last_heartbeat = Instant::now();
                        }
                    }
                    Ok(ReplicationMessage::WalAck { follower_id: fid, lsn: ack_lsn }) => {
                        let mut guard = followers.write().await;
                        if let Some(state) = guard.get_mut(&fid) {
                            state.last_lsn = ack_lsn;
                            state.last_heartbeat = Instant::now();
                        }
                    }
                    Ok(other) => {
                        tracing::warn!(follower = %follower_id, message = ?other, "replication: unexpected message from follower, dropping follower");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(follower = %follower_id, error = %e, "replication: follower connection closed");
                        break;
                    }
                    },
                }
            }
        }
    }
    reader_task.abort();

    // Mark the follower as disconnected.
    {
        let mut guard = followers.write().await;
        if let Some(state) = guard.get_mut(&follower_id) {
            state.is_alive = false;
        }
    }

    Ok(())
}

// ─── Follower ────────────────────────────────────────────────────────────────

/// Manages inbound replication on a follower node.
///
/// Connects to the leader's replication port, processes incoming WAL entries,
/// and monitors leader liveness via heartbeat timeouts.
pub struct FollowerReplicationManager {
    config: ReplicationConfig,
    /// The follower's current applied LSN
    current_lsn: Arc<AtomicU64>,
    /// Set to `true` while the leader connection is considered healthy
    is_leader_alive: Arc<AtomicBool>,
    /// Timestamp of the most recent heartbeat (or connection time)
    last_heartbeat: Arc<RwLock<Instant>>,
    /// Local graph to which WAL entries are applied
    graph: Arc<ConcurrentGraph>,
    /// Set once the server promoted this node to leader.
    is_promoted: Arc<AtomicBool>,
    /// When set, connect to the leader over TLS.
    leader_tls: Option<LeaderTls>,
    /// Set by [`stop`](Self::stop): stop following and do not reconnect.
    stopped: Arc<AtomicBool>,
    /// The task that receives from (and reconnects to) the leader.
    receiver: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Index / schema maintenance for applied changes.
    apply_hook: Arc<std::sync::OnceLock<Arc<dyn ReplicaApply>>>,
}

impl FollowerReplicationManager {
    /// Create a new follower replication manager with the given configuration.
    ///
    /// `is_leader_alive` starts as `false`; it becomes `true` only after the
    /// first successful handshake.
    pub fn new(config: ReplicationConfig) -> Self {
        Self {
            config,
            current_lsn: Arc::new(AtomicU64::new(0)),
            // Not connected yet, so the leader is not alive.
            is_leader_alive: Arc::new(AtomicBool::new(false)),
            last_heartbeat: Arc::new(RwLock::new(Instant::now())),
            graph: Arc::new(ConcurrentGraph::new()),
            is_promoted: Arc::new(AtomicBool::new(false)),
            leader_tls: None,
            stopped: Arc::new(AtomicBool::new(false)),
            receiver: std::sync::Mutex::new(None),
            apply_hook: Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Create a follower manager that applies WAL entries to an existing shared graph.
    pub fn with_concurrent_graph(config: ReplicationConfig, graph: Arc<ConcurrentGraph>) -> Self {
        Self {
            config,
            current_lsn: Arc::new(AtomicU64::new(0)),
            is_leader_alive: Arc::new(AtomicBool::new(false)),
            last_heartbeat: Arc::new(RwLock::new(Instant::now())),
            graph,
            is_promoted: Arc::new(AtomicBool::new(false)),
            leader_tls: None,
            stopped: Arc::new(AtomicBool::new(false)),
            receiver: std::sync::Mutex::new(None),
            apply_hook: Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Connect to the leader over TLS, verifying its certificate.
    pub fn with_leader_tls(mut self, tls: LeaderTls) -> Self {
        self.leader_tls = Some(tls);
        self
    }

    /// Return `true` if this follower has been instructed to promote to leader.
    pub fn is_promoted(&self) -> bool {
        self.is_promoted.load(Ordering::SeqCst)
    }

    /// Return a reference to the shared graph held by this follower.
    pub fn graph(&self) -> Arc<ConcurrentGraph> {
        Arc::clone(&self.graph)
    }

    /// Return `true` if the leader is considered reachable.
    pub fn is_leader_alive(&self) -> bool {
        self.is_leader_alive.load(Ordering::SeqCst)
    }

    /// Return the highest LSN this follower has applied.
    pub fn get_current_lsn(&self) -> u64 {
        self.current_lsn.load(Ordering::SeqCst)
    }

    /// Return a statistics snapshot for this node.
    pub fn get_stats(&self) -> ReplicationStats {
        ReplicationStats {
            role: "Follower".to_string(),
            node_id: self.config.node_id.clone(),
            current_lsn: self.get_current_lsn(),
            follower_count: 0,
            is_leader_alive: self.is_leader_alive(),
        }
    }

    /// Connect to the leader (plain TCP or TLS) and complete nothing else.
    async fn connect(&self) -> Result<Box<dyn ReplicationStream>, ReplicationError> {
        let leader_address = self
            .config
            .leader_address
            .clone()
            .ok_or(ReplicationError::LeaderNotConfigured)?;
        let tcp = TcpStream::connect(&leader_address)
            .await
            .map_err(|e| ReplicationError::ConnectionFailed(e.to_string()))?;
        Ok(match &self.leader_tls {
            None => Box::new(tcp),
            Some(tls) => {
                let connector = tokio_rustls::TlsConnector::from(Arc::clone(&tls.client_config));
                let handshake = connector.connect(tls.server_name.clone(), tcp);
                let stream = timeout(REPLICATION_TLS_HANDSHAKE_TIMEOUT, handshake)
                    .await
                    .map_err(|_| {
                        ReplicationError::ConnectionFailed("TLS handshake timed out".to_string())
                    })?
                    .map_err(|e| {
                        ReplicationError::ConnectionFailed(format!(
                            "TLS handshake with the leader failed: {e}"
                        ))
                    })?;
                Box::new(stream)
            }
        })
    }

    /// Connect to the leader and start receiving WAL entries.
    ///
    /// Returns an error when the first connection attempt fails. Once
    /// connected, a background task applies the leader's snapshot and WAL
    /// stream and reconnects whenever the connection drops (each new
    /// connection starts with a fresh snapshot), until [`stop`](Self::stop).
    pub async fn start(self: &Arc<Self>) -> Result<(), ReplicationError> {
        let stream = self.connect().await?;
        self.spawn_receiver(Some(stream));
        Ok(())
    }

    /// Like [`start`](Self::start), but keeps retrying until the leader is
    /// reachable instead of failing on the first attempt.
    pub fn start_with_retry(self: &Arc<Self>) {
        self.spawn_receiver(None);
    }

    /// Stop following the leader: close the connection, stop reconnecting and
    /// wait until no more replicated changes are being applied.
    pub async fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        let handle = self
            .receiver
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(handle) = handle {
            // The receive loop applies each message synchronously, so aborting
            // at an await point never leaves a message half-applied.
            handle.abort();
            let _ = handle.await;
        }
        self.is_leader_alive.store(false, Ordering::SeqCst);
    }

    /// Keep indexes and schema up to date as changes are applied. Only the
    /// first call has an effect.
    pub fn set_apply_hook(&self, hook: Arc<dyn ReplicaApply>) {
        let _ = self.apply_hook.set(hook);
    }

    /// Record that this node was promoted to leader.
    pub fn mark_promoted(&self) {
        self.is_promoted.store(true, Ordering::SeqCst);
    }

    fn spawn_receiver(self: &Arc<Self>, first: Option<Box<dyn ReplicationStream>>) {
        let this = Arc::clone(self);
        let handle = tokio::spawn(async move {
            let mut next = first;
            let mut announced_wait = false;
            loop {
                if this.stopped.load(Ordering::SeqCst) || this.is_promoted() {
                    break;
                }
                let stream = match next.take() {
                    Some(s) => s,
                    None => match this.connect().await {
                        Ok(s) => s,
                        Err(e) => {
                            if !announced_wait {
                                eprintln!("Replication: cannot reach the leader ({e}); retrying");
                                announced_wait = true;
                            }
                            tokio::time::sleep(FOLLOWER_RECONNECT_DELAY).await;
                            continue;
                        }
                    },
                };
                if announced_wait {
                    println!("Replication: connected to the leader");
                    announced_wait = false;
                }
                if let Err(e) = run_follower_receive_loop(
                    stream,
                    this.config.clone(),
                    Arc::clone(&this.current_lsn),
                    Arc::clone(&this.is_leader_alive),
                    Arc::clone(&this.last_heartbeat),
                    Arc::clone(&this.graph),
                    this.apply_hook.get().cloned(),
                )
                .await
                {
                    eprintln!("Follower receive loop error: {}", e);
                }
                this.is_leader_alive.store(false, Ordering::SeqCst);
                tokio::time::sleep(FOLLOWER_RECONNECT_DELAY).await;
            }
        });
        *self.receiver.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);

        // Heartbeat watchdog: flips `is_leader_alive` to false when no
        // heartbeat arrived within `heartbeat_timeout_secs`.
        let is_leader_alive = Arc::clone(&self.is_leader_alive);
        let last_heartbeat = Arc::clone(&self.last_heartbeat);
        let stopped = Arc::clone(&self.stopped);
        let timeout_secs = self.config.heartbeat_timeout_secs;
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(1));
            while !stopped.load(Ordering::SeqCst) {
                ticker.tick().await;
                let last = *last_heartbeat.read().await;
                if last.elapsed() > Duration::from_secs(timeout_secs) {
                    is_leader_alive.store(false, Ordering::SeqCst);
                }
            }
        });
    }
}

/// Apply a single WAL entry to the local graph (best-effort; errors are logged).
///
/// Uses `ConcurrentGraph` directly — no async lock needed.
pub(crate) fn apply_wal_entry(graph: &ConcurrentGraph, entry: &WalEntryData) {
    match entry {
        WalEntryData::CreateNode { node_id, labels } => {
            graph.create_node_with_id_and_labels(*node_id, labels.clone());
        }
        WalEntryData::DeleteNode { node_id } => {
            graph.delete_node(*node_id);
        }
        WalEntryData::CreateEdge {
            edge_id,
            from,
            to,
            label,
        } => {
            // Keep the leader's edge ID so later SetProperty/DeleteEdge entries
            // (which address edges by ID) hit the same edge on the follower.
            if let Err(e) = graph.create_edge_with_id(*edge_id, *from, *to, label) {
                eprintln!("WAL apply: create_edge failed: {}", e);
            }
        }
        WalEntryData::DeleteEdge { edge_id } => {
            graph.delete_edge(*edge_id);
        }
        WalEntryData::SetProperty {
            target_id,
            is_node,
            key,
            value,
        } => {
            let prop_val = property_value_from_wal_string(value);

            if *is_node {
                graph.set_node_property(*target_id, key, prop_val);
            } else {
                graph.set_edge_property(*target_id, key, prop_val);
            }
        }
        WalEntryData::RemoveProperty {
            target_id,
            is_node,
            key,
        } => {
            if *is_node {
                graph.remove_node_property(*target_id, key);
            } else {
                graph.remove_edge_property(*target_id, key);
            }
        }
        WalEntryData::AddLabel { node_id, label } => {
            graph.add_node_label(*node_id, label.clone());
        }
        WalEntryData::RemoveLabel { node_id, label } => {
            graph.remove_node_label(*node_id, label);
        }
        // Schema lives in the server's managers, not the graph: callers that
        // own managers apply it (see `ReplicaApply`, WAL recovery).
        WalEntryData::SetSchema { .. } => {}
    }
}

/// Nodes whose index entries an applied entry may have changed.
pub(crate) fn touched_node(entry: &WalEntryData) -> Option<u64> {
    match entry {
        WalEntryData::CreateNode { node_id, .. }
        | WalEntryData::DeleteNode { node_id }
        | WalEntryData::AddLabel { node_id, .. }
        | WalEntryData::RemoveLabel { node_id, .. } => Some(*node_id),
        WalEntryData::SetProperty {
            target_id,
            is_node: true,
            ..
        }
        | WalEntryData::RemoveProperty {
            target_id,
            is_node: true,
            ..
        } => Some(*target_id),
        _ => None,
    }
}

/// What a follower does besides changing the graph: keep the server's
/// indexes and schema in line with the replicated changes.
pub trait ReplicaApply: Send + Sync {
    /// `entry` (from the WAL stream) was just applied to `graph`.
    fn applied(&self, graph: &ConcurrentGraph, entry: &WalEntryData);
    /// A snapshot just replaced `graph`; `schema` came with it.
    fn snapshot_applied(&self, graph: &ConcurrentGraph, schema: &crate::schema::Schema);
}

/// A byte stream to the leader: plain TCP or TLS.
trait ReplicationStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> ReplicationStream for T {}

/// Core receive loop executed in a background task on the follower.
async fn run_follower_receive_loop(
    stream: Box<dyn ReplicationStream>,
    config: ReplicationConfig,
    current_lsn: Arc<AtomicU64>,
    is_leader_alive: Arc<AtomicBool>,
    last_heartbeat: Arc<RwLock<Instant>>,
    graph: Arc<ConcurrentGraph>,
    hook: Option<Arc<dyn ReplicaApply>>,
) -> Result<(), ReplicationError> {
    let (mut reader, mut writer) = tokio::io::split(stream);

    // ── Handshake ────────────────────────────────────────────────────────────
    let my_lsn = current_lsn.load(Ordering::SeqCst);
    send_message(
        &mut writer,
        &ReplicationMessage::Handshake {
            follower_id: config.node_id.clone(),
            current_lsn: my_lsn,
            auth_token: config.shared_secret.clone(),
        },
    )
    .await?;

    let ack = recv_message(&mut reader).await?;
    match ack {
        ReplicationMessage::HandshakeAck { .. } => {
            // Connection established; the leader is alive.
            is_leader_alive.store(true, Ordering::SeqCst);
            *last_heartbeat.write().await = Instant::now();
        }
        ReplicationMessage::Unauthorized { reason } => {
            return Err(ReplicationError::ConnectionFailed(format!(
                "replication handshake rejected by leader: {}",
                reason
            )));
        }
        other => {
            return Err(ReplicationError::ConnectionFailed(format!(
                "Expected HandshakeAck, got {:?}",
                other
            )));
        }
    }

    // ── Main receive loop ────────────────────────────────────────────────────
    loop {
        let msg = match recv_message(&mut reader).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, "replication: connection to leader lost");
                eprintln!("Replication: connection to leader lost: {}", e);
                is_leader_alive.store(false, Ordering::SeqCst);
                break;
            }
        };

        match msg {
            ReplicationMessage::Heartbeat {
                leader_id: _,
                timestamp: _,
                lsn: _,
            } => {
                // Update liveness tracking.
                is_leader_alive.store(true, Ordering::SeqCst);
                *last_heartbeat.write().await = Instant::now();

                let ack = ReplicationMessage::HeartbeatAck {
                    follower_id: config.node_id.clone(),
                    lsn: current_lsn.load(Ordering::SeqCst),
                };
                if send_message(&mut writer, &ack).await.is_err() {
                    break;
                }
            }

            ReplicationMessage::WalEntry { lsn, entry } => {
                // Apply the WAL entry to the local graph (best-effort).
                apply_wal_entry(&graph, &entry);
                if let Some(hook) = &hook {
                    hook.applied(&graph, &entry);
                }
                current_lsn.store(lsn, Ordering::SeqCst);
                *last_heartbeat.write().await = Instant::now();

                let ack = ReplicationMessage::WalAck {
                    follower_id: config.node_id.clone(),
                    lsn,
                };
                if send_message(&mut writer, &ack).await.is_err() {
                    break;
                }
            }

            ReplicationMessage::Snapshot { data, lsn } => {
                // Replace the local graph with the leader's: anything the
                // follower kept from before (its data file, or changes made
                // while it was disconnected) is out of date.
                match serde_json::from_slice::<Vec<WalEntryData>>(&data) {
                    Ok(entries) => {
                        graph.clear();
                        let mut schema = crate::schema::Schema::default();
                        for entry in entries {
                            match entry {
                                WalEntryData::SetSchema { schema: s } => schema = s,
                                other => apply_wal_entry(&graph, &other),
                            }
                        }
                        if let Some(hook) = &hook {
                            hook.snapshot_applied(&graph, &schema);
                        }
                        current_lsn.store(lsn, Ordering::SeqCst);
                        *last_heartbeat.write().await = Instant::now();
                        tracing::info!(
                            nodes = graph.node_count(),
                            edges = graph.edge_count(),
                            lsn,
                            "replication: applied snapshot from leader"
                        );
                    }
                    Err(e) => {
                        eprintln!("Snapshot deserialization error: {}", e);
                    }
                }
            }

            ReplicationMessage::PromoteToLeader => {
                // Promotion must also save the data file, open the WAL and
                // start serving replication, which only the server's
                // `promote` request does. Older versions sent this message.
                tracing::warn!(
                    "replication: ignoring PromoteToLeader; promote with `maharit admin promote-to-leader`"
                );
            }

            ReplicationMessage::Shutdown => {
                is_leader_alive.store(false, Ordering::SeqCst);
                break;
            }

            // Unexpected messages are silently ignored.
            _ => {}
        }
    }

    Ok(())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use maharit_core::PropertyValue;

    // 1. Verify default config values
    #[test]
    fn test_replication_config_default() {
        let config = ReplicationConfig::default();
        assert_eq!(config.role, NodeRole::Leader);
        assert_eq!(config.node_id, "node-1");
        assert_eq!(config.replication_bind_address, "127.0.0.1:7688");
        assert!(config.leader_address.is_none());
        assert_eq!(config.heartbeat_interval_secs, 1);
        assert_eq!(config.heartbeat_timeout_secs, 5);
    }

    // 2. Verify NodeRole serialises and round-trips correctly
    #[test]
    fn test_node_role_serialization() {
        let leader = NodeRole::Leader;
        let json = serde_json::to_string(&leader).unwrap();
        assert_eq!(json, "\"Leader\"");
        let back: NodeRole = serde_json::from_str(&json).unwrap();
        assert_eq!(back, NodeRole::Leader);

        let follower = NodeRole::Follower;
        let json = serde_json::to_string(&follower).unwrap();
        assert_eq!(json, "\"Follower\"");
        let back: NodeRole = serde_json::from_str(&json).unwrap();
        assert_eq!(back, NodeRole::Follower);
    }

    // 3. Verify all WalEntryData variants can be constructed
    #[test]
    fn test_wal_entry_data_variants() {
        let create_node = WalEntryData::CreateNode {
            node_id: 1,
            labels: vec!["Person".to_string()],
        };
        let delete_node = WalEntryData::DeleteNode { node_id: 1 };
        let create_edge = WalEntryData::CreateEdge {
            edge_id: 10,
            from: 1,
            to: 2,
            label: "KNOWS".to_string(),
        };
        let delete_edge = WalEntryData::DeleteEdge { edge_id: 10 };
        let set_prop = WalEntryData::SetProperty {
            target_id: 1,
            is_node: true,
            key: "name".to_string(),
            value: "Alice".to_string(),
        };

        // Serialise each variant to ensure they are well-formed.
        for entry in [create_node, delete_node, create_edge, delete_edge, set_prop] {
            let json = serde_json::to_string(&entry).unwrap();
            assert!(!json.is_empty());
        }
    }

    // 4. Verify all ReplicationMessage variants serialise/deserialise
    #[test]
    fn test_replication_message_serialization() {
        let messages = vec![
            ReplicationMessage::Handshake {
                follower_id: "follower-1".to_string(),
                current_lsn: 0,
                auth_token: None,
            },
            ReplicationMessage::HandshakeAck {
                leader_id: "leader-1".to_string(),
            },
            ReplicationMessage::Heartbeat {
                leader_id: "leader-1".to_string(),
                timestamp: 1_000_000,
                lsn: 42,
            },
            ReplicationMessage::HeartbeatAck {
                follower_id: "follower-1".to_string(),
                lsn: 42,
            },
            ReplicationMessage::WalEntry {
                lsn: 1,
                entry: WalEntryData::CreateNode {
                    node_id: 99,
                    labels: vec!["Test".to_string()],
                },
            },
            ReplicationMessage::WalAck {
                follower_id: "follower-1".to_string(),
                lsn: 1,
            },
            ReplicationMessage::Shutdown,
        ];

        for msg in messages {
            let json = serde_json::to_string(&msg).unwrap();
            let back: ReplicationMessage = serde_json::from_str(&json).unwrap();
            // Verify the round-trip produces the same JSON (structural equality
            // via re-serialisation, since ReplicationMessage is not PartialEq).
            let json2 = serde_json::to_string(&back).unwrap();
            assert_eq!(json, json2);
        }
    }

    // 5. Verify LeaderReplicationManager::new() constructs successfully
    #[test]
    fn test_leader_manager_creation() {
        let config = ReplicationConfig::default();
        let manager = LeaderReplicationManager::new(config);
        assert_eq!(manager.lsn.load(Ordering::SeqCst), 0);
        assert!(!manager.shutdown.load(Ordering::SeqCst));
    }

    // 6. Verify FollowerReplicationManager::new() constructs successfully
    #[test]
    fn test_follower_manager_creation() {
        let config = ReplicationConfig {
            role: NodeRole::Follower,
            node_id: "follower-1".to_string(),
            leader_address: Some("127.0.0.1:7688".to_string()),
            ..ReplicationConfig::default()
        };
        let manager = FollowerReplicationManager::new(config);
        assert_eq!(manager.get_current_lsn(), 0);
    }

    // 7. Verify get_stats() returns correct data for a leader
    #[test]
    fn test_leader_stats() {
        let config = ReplicationConfig {
            node_id: "leader-node".to_string(),
            ..ReplicationConfig::default()
        };
        let manager = LeaderReplicationManager::new(config);
        let stats = manager.get_stats();
        assert_eq!(stats.role, "Leader");
        assert_eq!(stats.node_id, "leader-node");
        assert_eq!(stats.current_lsn, 0);
        assert_eq!(stats.follower_count, 0);
        assert!(stats.is_leader_alive);
    }

    // 8. Verify is_leader_alive() returns false before any connection
    #[test]
    fn test_follower_is_leader_alive_initial() {
        let config = ReplicationConfig {
            role: NodeRole::Follower,
            node_id: "follower-1".to_string(),
            leader_address: Some("127.0.0.1:7688".to_string()),
            ..ReplicationConfig::default()
        };
        let manager = FollowerReplicationManager::new(config);
        // Not yet connected, so the leader should not be considered alive.
        assert!(!manager.is_leader_alive());
    }

    // 9. Verify append_wal_entry increments LSN monotonically
    #[tokio::test]
    async fn test_wal_entry_append_increments_lsn() {
        let config = ReplicationConfig::default();
        let manager = LeaderReplicationManager::new(config);

        assert_eq!(manager.lsn.load(Ordering::SeqCst), 0);

        let lsn1 = manager
            .append_wal_entry(WalEntryData::CreateNode {
                node_id: 1,
                labels: vec!["Person".to_string()],
            })
            .await;
        assert_eq!(lsn1, 1);

        let lsn2 = manager
            .append_wal_entry(WalEntryData::CreateNode {
                node_id: 2,
                labels: vec!["City".to_string()],
            })
            .await;
        assert_eq!(lsn2, 2);

        let lsn3 = manager
            .append_wal_entry(WalEntryData::CreateEdge {
                edge_id: 1,
                from: 1,
                to: 2,
                label: "LIVES_IN".to_string(),
            })
            .await;
        assert_eq!(lsn3, 3);

        assert_eq!(manager.lsn.load(Ordering::SeqCst), 3);
    }

    // 10. Verify apply_wal_entry creates nodes, edges, and sets properties
    #[test]
    fn test_apply_wal_entry_create_node_multilabel() {
        use maharit_core::ConcurrentGraph;
        let graph = Arc::new(ConcurrentGraph::new());

        apply_wal_entry(
            &graph,
            &WalEntryData::CreateNode {
                node_id: 0,
                labels: vec!["Person".to_string(), "Employee".to_string()],
            },
        );

        assert_eq!(graph.node_count(), 1);
        let labels = graph.with_node(0, |n| n.labels.clone()).unwrap();
        assert!(labels.contains(&"Person".to_string()));
        assert!(labels.contains(&"Employee".to_string()));
    }

    #[test]
    fn test_apply_wal_entry_set_property() {
        use maharit_core::ConcurrentGraph;
        let graph = Arc::new(ConcurrentGraph::new());

        apply_wal_entry(
            &graph,
            &WalEntryData::CreateNode {
                node_id: 0,
                labels: vec!["Person".to_string()],
            },
        );

        apply_wal_entry(
            &graph,
            &WalEntryData::SetProperty {
                target_id: 0,
                is_node: true,
                key: "name".to_string(),
                value: "\"Alice\"".to_string(),
            },
        );

        let prop = graph.with_node(0, |n| n.properties.get("name").cloned());
        assert_eq!(
            prop.flatten(),
            Some(PropertyValue::String("Alice".to_string()))
        );
    }

    #[test]
    fn test_apply_wal_entry_updates_and_edge_ids() {
        use maharit_core::{ConcurrentGraph, GraphBackend};
        let graph = Arc::new(ConcurrentGraph::new());
        for id in [0, 1] {
            apply_wal_entry(
                &graph,
                &WalEntryData::CreateNode {
                    node_id: id,
                    labels: vec!["N".to_string()],
                },
            );
        }
        // The follower must keep the leader's edge ID even when it differs
        // from what its own counter would allocate.
        apply_wal_entry(
            &graph,
            &WalEntryData::CreateEdge {
                edge_id: 7,
                from: 0,
                to: 1,
                label: "R".to_string(),
            },
        );
        assert!(graph.get_edge(7).is_some());

        let set = |target_id, is_node, key: &str, value: &str| WalEntryData::SetProperty {
            target_id,
            is_node,
            key: key.to_string(),
            value: value.to_string(),
        };
        apply_wal_entry(&graph, &set(7, false, "w", "1.0"));
        assert_eq!(
            graph.get_edge(7).unwrap().properties.get("w"),
            Some(&PropertyValue::Float(1.0))
        );
        apply_wal_entry(&graph, &set(0, true, "name", "\"a\""));
        apply_wal_entry(
            &graph,
            &WalEntryData::RemoveProperty {
                target_id: 0,
                is_node: true,
                key: "name".to_string(),
            },
        );
        assert_eq!(graph.get_node_property(0, "name"), None);

        apply_wal_entry(
            &graph,
            &WalEntryData::AddLabel {
                node_id: 0,
                label: "Q".to_string(),
            },
        );
        apply_wal_entry(
            &graph,
            &WalEntryData::RemoveLabel {
                node_id: 0,
                label: "N".to_string(),
            },
        );
        let node = graph.get_node(0).unwrap();
        assert!(node.has_label("Q") && !node.has_label("N"));
    }

    #[test]
    fn test_apply_wal_entry_delete_node() {
        use maharit_core::ConcurrentGraph;
        let graph = Arc::new(ConcurrentGraph::new());

        apply_wal_entry(
            &graph,
            &WalEntryData::CreateNode {
                node_id: 0,
                labels: vec!["N".to_string()],
            },
        );

        apply_wal_entry(&graph, &WalEntryData::DeleteNode { node_id: 0 });

        assert_eq!(graph.node_count(), 0);
    }

    #[test]
    fn test_follower_manager_with_graph() {
        use maharit_core::ConcurrentGraph;
        let shared_graph = Arc::new(ConcurrentGraph::new());
        let config = ReplicationConfig {
            role: NodeRole::Follower,
            node_id: "follower-1".to_string(),
            leader_address: Some("127.0.0.1:7699".to_string()),
            ..ReplicationConfig::default()
        };
        let manager =
            FollowerReplicationManager::with_concurrent_graph(config, Arc::clone(&shared_graph));
        assert!(Arc::ptr_eq(&manager.graph(), &shared_graph));
    }

    // 14. Verify handshake includes current_lsn in serialization
    #[test]
    fn test_handshake_with_lsn_serialization() {
        let msg = ReplicationMessage::Handshake {
            follower_id: "f1".to_string(),
            current_lsn: 42,
            auth_token: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ReplicationMessage = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&back).unwrap();
        assert_eq!(json, json2);
        assert!(json.contains("42"));
    }

    // 15. Verify backward-compat: Handshake without current_lsn defaults to 0
    #[test]
    fn test_handshake_backward_compat_default_lsn() {
        let old_json = r#"{"Handshake":{"follower_id":"f1"}}"#;
        let msg: ReplicationMessage = serde_json::from_str(old_json).unwrap();
        match msg {
            ReplicationMessage::Handshake { current_lsn, .. } => {
                assert_eq!(current_lsn, 0);
            }
            _ => panic!("Expected Handshake"),
        }
    }

    // 16. Verify Snapshot and PromoteToLeader message round-trip
    #[test]
    fn test_new_message_variants_serialize() {
        let snapshot = ReplicationMessage::Snapshot {
            data: b"hello".to_vec(),
            lsn: 7,
        };
        let promote = ReplicationMessage::PromoteToLeader;

        for msg in [snapshot, promote] {
            let json = serde_json::to_string(&msg).unwrap();
            let back: ReplicationMessage = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&back).unwrap();
            assert_eq!(json, json2);
        }
    }

    // 17. Verify graph_to_snapshot_entries captures nodes and properties
    #[tokio::test]
    async fn test_graph_to_snapshot_entries() {
        let graph = Arc::new(RwLock::new(Graph::new()));
        {
            let mut g = graph.write().await;
            let id = g.create_node_with_labels(vec!["Person".to_string(), "Employee".to_string()]);
            g.get_node_mut(id)
                .unwrap()
                .set_property("name", PropertyValue::String("Alice".to_string()));
            let b = g.create_node_with_labels(vec!["City".to_string()]);
            g.create_edge(id, b, "LIVES_IN").unwrap();
        }

        let entries = graph_to_snapshot_entries(&graph).await;
        // Should have: 2 CreateNode + 1 SetProperty + 1 CreateEdge = 4
        assert!(entries.len() >= 4);
        let create_nodes = entries
            .iter()
            .filter(|e| matches!(e, WalEntryData::CreateNode { .. }))
            .count();
        assert_eq!(create_nodes, 2);
        let create_edges = entries
            .iter()
            .filter(|e| matches!(e, WalEntryData::CreateEdge { .. }))
            .count();
        assert_eq!(create_edges, 1);
    }

    // 18. Verify FollowerReplicationManager.is_promoted() starts false
    #[test]
    fn test_follower_is_promoted_initial() {
        let config = ReplicationConfig {
            role: NodeRole::Follower,
            ..ReplicationConfig::default()
        };
        let manager = FollowerReplicationManager::new(config);
        assert!(!manager.is_promoted());
    }

    // 19. Verify LeaderReplicationManager::with_graph builds successfully
    #[test]
    fn test_leader_with_graph_constructor() {
        let graph = Arc::new(RwLock::new(Graph::new()));
        let config = ReplicationConfig::default();
        let manager = LeaderReplicationManager::with_graph(config, graph);
        assert!(manager.graph.is_some());
        assert_eq!(manager.lsn.load(Ordering::SeqCst), 0);
    }

    // ── Replication integration tests ────────────────────────────────────────
    //
    // These tests spin up in-process leader and follower nodes connected over
    // loopback TCP, then verify that data written to the leader is propagated
    // to the follower and queryable via its TcpServer.

    use crate::tcp_server::{Request, Response, ServerConfig, TcpServer};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Send one request and await one response over a raw TCP stream.
    async fn repl_send_recv(stream: &mut tokio::net::TcpStream, req: &Request) -> Response {
        let json = serde_json::to_vec(req).unwrap();
        let len = json.len() as u32;
        stream.write_all(&len.to_be_bytes()).await.unwrap();
        stream.write_all(&json).await.unwrap();
        stream.flush().await.unwrap();

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let body_len = u32::from_be_bytes(len_buf) as usize;
        let mut body = vec![0u8; body_len];
        stream.read_exact(&mut body).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    /// Start a `TcpServer` on a random port and return the bound address.
    async fn start_query_server(
        graph: Arc<ConcurrentGraph>,
        repl: Option<Arc<LeaderReplicationManager>>,
    ) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let mut server = TcpServer::with_graph_arc(
            ServerConfig {
                bind_address: addr.to_string(),
                read_timeout: Duration::from_secs(5),
                write_timeout: Duration::from_secs(5),
                ..Default::default()
            },
            graph,
        );
        if let Some(r) = repl {
            server = server.with_replication(r);
        }

        tokio::spawn(async move {
            server.start_with_listener(listener).await.unwrap();
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        addr
    }

    /// Start a `LeaderReplicationManager` on a random port and return
    /// `(manager, replication_addr)`.
    async fn start_leader_replication() -> (Arc<LeaderReplicationManager>, std::net::SocketAddr) {
        let repl_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let repl_addr = repl_listener.local_addr().unwrap();

        let config = ReplicationConfig {
            role: NodeRole::Leader,
            node_id: "leader".to_string(),
            replication_bind_address: repl_addr.to_string(),
            leader_address: None,
            heartbeat_interval_secs: 1,
            heartbeat_timeout_secs: 5,
            shared_secret: None,
        };
        let manager = Arc::new(LeaderReplicationManager::new(config));
        manager.start_with_listener(repl_listener).await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        (manager, repl_addr)
    }

    /// Start a follower replication manager connecting to `leader_addr`.
    /// Returns `(manager, follower_graph)`.
    async fn start_follower_replication(
        leader_addr: std::net::SocketAddr,
    ) -> (Arc<FollowerReplicationManager>, Arc<ConcurrentGraph>) {
        let follower_graph = Arc::new(ConcurrentGraph::new());
        let config = ReplicationConfig {
            role: NodeRole::Follower,
            node_id: "follower".to_string(),
            replication_bind_address: "127.0.0.1:0".to_string(),
            leader_address: Some(leader_addr.to_string()),
            heartbeat_interval_secs: 1,
            heartbeat_timeout_secs: 5,
            shared_secret: None,
        };
        let manager = Arc::new(FollowerReplicationManager::with_concurrent_graph(
            config,
            Arc::clone(&follower_graph),
        ));
        manager.start().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        (manager, follower_graph)
    }

    #[test]
    fn test_replication_secret_eq() {
        assert!(replication_secret_eq("s3cr3t", "s3cr3t"));
        assert!(!replication_secret_eq("s3cr3t", "s3cr3x"));
        assert!(!replication_secret_eq("short", "longer"));
    }

    /// Start a leader replication listener with a shared secret.
    async fn start_leader_with_secret(
        secret: Option<&str>,
    ) -> (Arc<LeaderReplicationManager>, std::net::SocketAddr) {
        let repl_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let repl_addr = repl_listener.local_addr().unwrap();
        let config = ReplicationConfig {
            role: NodeRole::Leader,
            node_id: "leader".to_string(),
            replication_bind_address: repl_addr.to_string(),
            leader_address: None,
            heartbeat_interval_secs: 1,
            heartbeat_timeout_secs: 5,
            shared_secret: secret.map(|s| s.to_string()),
        };
        let manager = Arc::new(LeaderReplicationManager::new(config));
        manager.start_with_listener(repl_listener).await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        (manager, repl_addr)
    }

    #[tokio::test]
    async fn replication_rejects_wrong_secret() {
        let (_leader, addr) = start_leader_with_secret(Some("correct-horse")).await;
        let mut stream = TcpStream::connect(addr).await.unwrap();
        send_message(
            &mut stream,
            &ReplicationMessage::Handshake {
                follower_id: "rogue".to_string(),
                current_lsn: 0,
                auth_token: Some("wrong".to_string()),
            },
        )
        .await
        .unwrap();
        let resp = recv_message(&mut stream).await.unwrap();
        assert!(
            matches!(resp, ReplicationMessage::Unauthorized { .. }),
            "expected Unauthorized, got {:?}",
            resp
        );
    }

    #[tokio::test]
    async fn replication_rejects_missing_secret() {
        let (_leader, addr) = start_leader_with_secret(Some("correct-horse")).await;
        let mut stream = TcpStream::connect(addr).await.unwrap();
        send_message(
            &mut stream,
            &ReplicationMessage::Handshake {
                follower_id: "rogue".to_string(),
                current_lsn: 0,
                auth_token: None,
            },
        )
        .await
        .unwrap();
        let resp = recv_message(&mut stream).await.unwrap();
        assert!(matches!(resp, ReplicationMessage::Unauthorized { .. }));
    }

    #[tokio::test]
    async fn replication_accepts_correct_secret() {
        let (_leader, addr) = start_leader_with_secret(Some("correct-horse")).await;
        let mut stream = TcpStream::connect(addr).await.unwrap();
        send_message(
            &mut stream,
            &ReplicationMessage::Handshake {
                follower_id: "trusted".to_string(),
                current_lsn: 1, // non-zero so no snapshot is sent; expect a plain ack
                auth_token: Some("correct-horse".to_string()),
            },
        )
        .await
        .unwrap();
        let resp = recv_message(&mut stream).await.unwrap();
        assert!(
            matches!(resp, ReplicationMessage::HandshakeAck { .. }),
            "expected HandshakeAck, got {:?}",
            resp
        );
    }

    #[tokio::test]
    async fn replication_integration_follower_receives_created_node() {
        let (leader_repl, repl_addr) = start_leader_replication().await;
        let (_, follower_graph) = start_follower_replication(repl_addr).await;

        // Leader: start query server with replication wired in
        let leader_graph = Arc::new(ConcurrentGraph::new());
        let leader_addr =
            start_query_server(Arc::clone(&leader_graph), Some(Arc::clone(&leader_repl))).await;

        // Follower: start query server against the follower's graph
        let follower_addr = start_query_server(Arc::clone(&follower_graph), None).await;

        // Write to leader
        let mut lconn = tokio::net::TcpStream::connect(leader_addr).await.unwrap();
        repl_send_recv(
            &mut lconn,
            &Request::Query {
                query: "CREATE (n:Test {name: 'Alice'}) RETURN n".to_string(),
                params: HashMap::new(),
                tx_id: None,
                session_token: None,
            },
        )
        .await;

        // Wait for WAL propagation
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Read from follower
        let mut fconn = tokio::net::TcpStream::connect(follower_addr).await.unwrap();
        let resp = repl_send_recv(
            &mut fconn,
            &Request::Query {
                query: "MATCH (n:Test) RETURN n.name".to_string(),
                params: HashMap::new(),
                tx_id: None,
                session_token: None,
            },
        )
        .await;

        match resp {
            Response::Result { rows } => {
                assert!(
                    !rows.is_empty(),
                    "follower should have the node created on leader"
                );
                assert_eq!(
                    rows[0].get("n.name").and_then(|v| v.as_str()),
                    Some("Alice")
                );
            }
            other => panic!("expected Result from follower, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn replication_integration_follower_receives_node_properties() {
        let (leader_repl, repl_addr) = start_leader_replication().await;
        let (_, follower_graph) = start_follower_replication(repl_addr).await;

        let leader_graph = Arc::new(ConcurrentGraph::new());
        let leader_addr =
            start_query_server(Arc::clone(&leader_graph), Some(Arc::clone(&leader_repl))).await;
        let follower_addr = start_query_server(Arc::clone(&follower_graph), None).await;

        // Write node with string + integer + bool properties
        let mut lconn = tokio::net::TcpStream::connect(leader_addr).await.unwrap();
        repl_send_recv(
            &mut lconn,
            &Request::Query {
                query: "CREATE (n:Prop {s: 'hello', i: 99, b: true}) RETURN n".to_string(),
                params: HashMap::new(),
                tx_id: None,
                session_token: None,
            },
        )
        .await;

        tokio::time::sleep(Duration::from_millis(200)).await;

        let mut fconn = tokio::net::TcpStream::connect(follower_addr).await.unwrap();

        // String property
        let resp = repl_send_recv(
            &mut fconn,
            &Request::Query {
                query: "MATCH (n:Prop) RETURN n.s".to_string(),
                params: HashMap::new(),
                tx_id: None,
                session_token: None,
            },
        )
        .await;
        match resp {
            Response::Result { rows } => {
                assert!(!rows.is_empty());
                assert_eq!(rows[0].get("n.s").and_then(|v| v.as_str()), Some("hello"));
            }
            other => panic!("string property: {:?}", other),
        }

        // Integer property
        let resp = repl_send_recv(
            &mut fconn,
            &Request::Query {
                query: "MATCH (n:Prop) RETURN n.i".to_string(),
                params: HashMap::new(),
                tx_id: None,
                session_token: None,
            },
        )
        .await;
        match resp {
            Response::Result { rows } => {
                assert!(!rows.is_empty());
                assert_eq!(rows[0].get("n.i").and_then(|v| v.as_i64()), Some(99));
            }
            other => panic!("integer property: {:?}", other),
        }
    }

    #[tokio::test]
    async fn replication_integration_follower_receives_edge() {
        let (leader_repl, repl_addr) = start_leader_replication().await;
        let (_, follower_graph) = start_follower_replication(repl_addr).await;

        let leader_graph = Arc::new(ConcurrentGraph::new());
        let leader_addr =
            start_query_server(Arc::clone(&leader_graph), Some(Arc::clone(&leader_repl))).await;
        let follower_addr = start_query_server(Arc::clone(&follower_graph), None).await;

        // Create two nodes and an edge on the leader
        let mut lconn = tokio::net::TcpStream::connect(leader_addr).await.unwrap();
        for q in [
            "CREATE (n:Src {name: 'src'})",
            "CREATE (n:Dst {name: 'dst'})",
            "MATCH (a:Src {name: 'src'}), (b:Dst {name: 'dst'}) CREATE (a)-[:LINK]->(b)",
        ] {
            repl_send_recv(
                &mut lconn,
                &Request::Query {
                    query: q.to_string(),
                    params: HashMap::new(),
                    tx_id: None,
                    session_token: None,
                },
            )
            .await;
        }

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Traverse on follower
        let mut fconn = tokio::net::TcpStream::connect(follower_addr).await.unwrap();
        let resp = repl_send_recv(
            &mut fconn,
            &Request::Query {
                query: "MATCH (a:Src)-[:LINK]->(b:Dst) RETURN b.name".to_string(),
                params: HashMap::new(),
                tx_id: None,
                session_token: None,
            },
        )
        .await;
        match resp {
            Response::Result { rows } => {
                assert!(!rows.is_empty(), "follower should see the edge traversal");
                assert_eq!(rows[0].get("b.name").and_then(|v| v.as_str()), Some("dst"));
            }
            other => panic!("expected Result for edge traversal, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn replication_integration_follower_receives_deletion() {
        let (leader_repl, repl_addr) = start_leader_replication().await;
        let (_, follower_graph) = start_follower_replication(repl_addr).await;

        let leader_graph = Arc::new(ConcurrentGraph::new());
        let leader_addr =
            start_query_server(Arc::clone(&leader_graph), Some(Arc::clone(&leader_repl))).await;
        let follower_addr = start_query_server(Arc::clone(&follower_graph), None).await;

        let mut lconn = tokio::net::TcpStream::connect(leader_addr).await.unwrap();

        // Create then delete
        repl_send_recv(
            &mut lconn,
            &Request::Query {
                query: "CREATE (n:Del {name: 'gone'}) RETURN n".to_string(),
                params: HashMap::new(),
                tx_id: None,
                session_token: None,
            },
        )
        .await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        repl_send_recv(
            &mut lconn,
            &Request::Query {
                query: "MATCH (n:Del {name: 'gone'}) DETACH DELETE n".to_string(),
                params: HashMap::new(),
                tx_id: None,
                session_token: None,
            },
        )
        .await;

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Follower should return 0 rows
        let mut fconn = tokio::net::TcpStream::connect(follower_addr).await.unwrap();
        let resp = repl_send_recv(
            &mut fconn,
            &Request::Query {
                query: "MATCH (n:Del) RETURN n.name".to_string(),
                params: HashMap::new(),
                tx_id: None,
                session_token: None,
            },
        )
        .await;
        match resp {
            Response::Result { rows } => {
                assert!(
                    rows.is_empty(),
                    "deleted node should not appear on follower"
                );
            }
            other => panic!("expected Result, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn replication_integration_follower_and_server_share_same_graph() {
        let leader_addr_str = "127.0.0.1:0";
        let follower_graph = Arc::new(ConcurrentGraph::new());

        let config = ReplicationConfig {
            role: NodeRole::Follower,
            node_id: "f".to_string(),
            replication_bind_address: leader_addr_str.to_string(),
            leader_address: Some("127.0.0.1:1".to_string()), // dummy; not starting
            heartbeat_interval_secs: 1,
            heartbeat_timeout_secs: 5,
            shared_secret: None,
        };
        let follower_repl =
            FollowerReplicationManager::with_concurrent_graph(config, Arc::clone(&follower_graph));

        // The follower replication manager's graph and the TcpServer's graph must be the same Arc
        assert!(
            Arc::ptr_eq(&follower_repl.graph(), &follower_graph),
            "FollowerReplicationManager and TcpServer must share the same graph Arc"
        );
    }

    // 20. The obsolete PromoteToLeader message must not promote a follower: that
    // would accept writes without a WAL or a replication listener.
    /// Deterministic reproduction of task120: the follower's ACK frame arrives
    /// in two TCP segments and a WAL entry is broadcast in between. The leader
    /// must not lose the partially read frame (the old `select!` dropped the
    /// half-finished `recv_message` future, desynchronising the stream).
    #[tokio::test]
    async fn test_leader_survives_split_ack_frame() {
        use tokio::io::AsyncWriteExt;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let leader_addr = listener.local_addr().unwrap();
        let leader = LeaderReplicationManager::new(ReplicationConfig {
            heartbeat_interval_secs: 3600, // keep heartbeats out of the way
            ..ReplicationConfig::default()
        });
        leader.start_with_listener(listener).await.unwrap();

        let socket = TcpStream::connect(leader_addr).await.unwrap();
        let (mut reader, mut writer) = socket.into_split();
        send_message(
            &mut writer,
            &ReplicationMessage::Handshake {
                follower_id: "f-split".to_string(),
                current_lsn: 1,
                auth_token: None,
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            recv_message(&mut reader).await.unwrap(),
            ReplicationMessage::HandshakeAck { .. }
        ));
        // The first interval tick fires immediately: consume that heartbeat.
        assert!(matches!(
            recv_message(&mut reader).await.unwrap(),
            ReplicationMessage::Heartbeat { .. }
        ));

        // Write only the first half of a WalAck frame.
        let ack = serde_json::to_vec(&ReplicationMessage::WalAck {
            follower_id: "f-split".to_string(),
            lsn: 1,
        })
        .unwrap();
        let mut frame = (ack.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(&ack);
        let (head, tail) = frame.split_at(6);
        writer.write_all(head).await.unwrap();
        writer.flush().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // A WAL entry wins the select! while the ACK is half read.
        leader
            .append_wal_entry(WalEntryData::DeleteNode { node_id: 1 })
            .await;
        assert!(matches!(
            recv_message(&mut reader).await.unwrap(),
            ReplicationMessage::WalEntry { lsn: 1, .. }
        ));

        // Finish the ACK, then send another WAL entry: the stream must still be in sync.
        writer.write_all(tail).await.unwrap();
        writer.flush().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        leader
            .append_wal_entry(WalEntryData::DeleteNode { node_id: 2 })
            .await;
        let next = tokio::time::timeout(Duration::from_secs(2), recv_message(&mut reader))
            .await
            .expect("leader stopped streaming after a split ACK frame");
        assert!(matches!(
            next,
            Ok(ReplicationMessage::WalEntry { lsn: 2, .. })
        ));
        assert_eq!(leader.get_follower_count(), 1);
    }

    /// A single burst far larger than the per-follower queue (e.g. one
    /// statement creating thousands of elements) must not drop entries: the
    /// old broadcast channel overflowed at 1024 and the follower silently
    /// missed data (found by the randomized leader/follower diff test).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_wal_burst_larger_than_queue_is_not_dropped() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let leader_addr = listener.local_addr().unwrap();
        let leader = LeaderReplicationManager::new(ReplicationConfig::default());
        leader.start_with_listener(listener).await.unwrap();

        let follower_graph = Arc::new(ConcurrentGraph::new());
        let follower = FollowerReplicationManager::with_concurrent_graph(
            ReplicationConfig {
                role: NodeRole::Follower,
                node_id: "f-overflow".to_string(),
                replication_bind_address: "127.0.0.1:0".to_string(),
                leader_address: Some(leader_addr.to_string()),
                heartbeat_interval_secs: 1,
                heartbeat_timeout_secs: 5,
                shared_secret: None,
            },
            Arc::clone(&follower_graph),
        );
        let follower = Arc::new(follower);
        follower.start().await.unwrap();
        for _ in 0..50 {
            if leader.get_follower_count() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let n = (WAL_QUEUE_CAPACITY * 5) as u64;
        for id in 0..n {
            leader
                .append_wal_entry(WalEntryData::CreateNode {
                    node_id: id,
                    labels: vec!["B".to_string()],
                })
                .await;
        }
        for _ in 0..200 {
            if follower_graph.node_count() as u64 == n {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(follower_graph.node_count() as u64, n);
        assert_eq!(
            leader.get_follower_count(),
            1,
            "follower must stay connected"
        );
    }

    /// A burst of WAL entries must all reach a real follower. The leader used to
    /// read follower ACKs with a non-cancellation-safe future inside
    /// `tokio::select!`; whenever a WAL send won the race mid-read, bytes were
    /// lost, the stream desynchronised and the leader silently dropped the
    /// follower (task120).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_wal_burst_fully_replicated() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let leader_addr = listener.local_addr().unwrap();
        let leader = LeaderReplicationManager::new(ReplicationConfig::default());
        leader.start_with_listener(listener).await.unwrap();

        let follower_graph = Arc::new(ConcurrentGraph::new());
        let follower = FollowerReplicationManager::with_concurrent_graph(
            ReplicationConfig {
                role: NodeRole::Follower,
                node_id: "f-burst".to_string(),
                replication_bind_address: "127.0.0.1:0".to_string(),
                leader_address: Some(leader_addr.to_string()),
                heartbeat_interval_secs: 1,
                heartbeat_timeout_secs: 5,
                shared_secret: None,
            },
            Arc::clone(&follower_graph),
        );
        let follower = Arc::new(follower);
        follower.start().await.unwrap();

        // Wait until the leader has registered the follower.
        for _ in 0..50 {
            if leader.get_follower_count() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(leader.get_follower_count(), 1);

        const N: u64 = 500;
        for id in 0..N {
            leader
                .append_wal_entry(WalEntryData::CreateNode {
                    node_id: id,
                    labels: vec!["B".to_string()],
                })
                .await;
            if id % 10 == 0 {
                // Interleave with the follower's ACK traffic.
                tokio::task::yield_now().await;
                tokio::time::sleep(Duration::from_micros(200)).await;
            }
        }

        for _ in 0..100 {
            if follower_graph.node_count() as u64 == N {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(follower_graph.node_count() as u64, N);
        assert_eq!(follower.get_current_lsn(), N);
        assert_eq!(
            leader.get_follower_count(),
            1,
            "follower must stay connected"
        );
    }

    #[tokio::test]
    async fn test_promote_to_leader_message_is_ignored() {
        // Bind a mock leader listener on an ephemeral port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let leader_addr = listener.local_addr().unwrap();

        // Start a follower pointing at the mock leader
        let manager = Arc::new(FollowerReplicationManager::with_concurrent_graph(
            ReplicationConfig {
                role: NodeRole::Follower,
                node_id: "f-promote".to_string(),
                replication_bind_address: "127.0.0.1:0".to_string(),
                leader_address: Some(leader_addr.to_string()),
                heartbeat_interval_secs: 1,
                heartbeat_timeout_secs: 5,
                shared_secret: None,
            },
            Arc::new(ConcurrentGraph::new()),
        ));
        manager.start().await.unwrap();

        // Spawn mock leader: accept connection, do handshake, then send PromoteToLeader
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let (mut reader, mut writer) = socket.into_split();
            // Read the Handshake from the follower
            let _ = recv_message(&mut reader).await.unwrap();
            // Acknowledge the handshake so is_leader_alive flips to true
            send_message(
                &mut writer,
                &ReplicationMessage::HandshakeAck {
                    leader_id: "mock-leader".to_string(),
                },
            )
            .await
            .unwrap();
            // Wait longer than the first assertion below (200 ms), then promote.
            // This ensures is_leader_alive is still true when the test first checks it.
            tokio::time::sleep(Duration::from_millis(200)).await;
            send_message(&mut writer, &ReplicationMessage::PromoteToLeader)
                .await
                .unwrap();
            // Keep socket alive long enough for the follower to process the message
            tokio::time::sleep(Duration::from_millis(300)).await;
        });

        // Wait for handshake to complete (is_leader_alive → true).
        // PromoteToLeader won't arrive for another ~200 ms, so this is safe.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            manager.is_leader_alive(),
            "leader should be alive after handshake"
        );
        assert!(!manager.is_promoted(), "should not be promoted yet");

        // Wait for the PromoteToLeader message to be processed (arrives at ~200 ms).
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !manager.is_promoted(),
            "PromoteToLeader over the replication channel must be ignored"
        );
        assert!(manager.is_leader_alive(), "the follower keeps following");
    }

    // 21. Verify that the heartbeat watchdog marks the leader as dead when no heartbeat arrives
    #[tokio::test]
    async fn test_heartbeat_timeout_detection() {
        // Use a short timeout so the test finishes quickly
        let timeout_secs = 2u64;

        // Bind a mock leader listener
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let leader_addr = listener.local_addr().unwrap();

        // Start a follower with the short heartbeat timeout
        let manager = Arc::new(FollowerReplicationManager::with_concurrent_graph(
            ReplicationConfig {
                role: NodeRole::Follower,
                node_id: "f-timeout".to_string(),
                replication_bind_address: "127.0.0.1:0".to_string(),
                leader_address: Some(leader_addr.to_string()),
                heartbeat_interval_secs: 1,
                heartbeat_timeout_secs: timeout_secs,
                shared_secret: None,
            },
            Arc::new(ConcurrentGraph::new()),
        ));
        manager.start().await.unwrap();

        // Spawn mock leader: accept, handshake, then go silent (no heartbeats)
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let (mut reader, mut writer) = socket.into_split();
            // Read the Handshake
            let _ = recv_message(&mut reader).await.unwrap();
            // Reply with HandshakeAck so the follower marks the leader as alive
            send_message(
                &mut writer,
                &ReplicationMessage::HandshakeAck {
                    leader_id: "mock-leader".to_string(),
                },
            )
            .await
            .unwrap();
            // Hold the connection open but send no further heartbeats
            tokio::time::sleep(Duration::from_secs(timeout_secs + 5)).await;
        });

        // After handshake the leader should be considered alive
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            manager.is_leader_alive(),
            "leader should be alive after handshake"
        );

        // After heartbeat_timeout_secs + one watchdog-tick second the leader should be dead
        tokio::time::sleep(Duration::from_secs(timeout_secs + 1)).await;
        assert!(
            !manager.is_leader_alive(),
            "leader should be marked dead after heartbeat timeout"
        );
    }

    // ── TLS ──────────────────────────────────────────────────────────────────

    fn tls_config(role: NodeRole, bind: &str, leader: Option<String>) -> ReplicationConfig {
        ReplicationConfig {
            role,
            node_id: "tls-node".to_string(),
            replication_bind_address: bind.to_string(),
            leader_address: leader,
            heartbeat_interval_secs: 1,
            heartbeat_timeout_secs: 5,
            shared_secret: Some("tls-secret".to_string()),
        }
    }

    fn leader_tls(ca: &str) -> LeaderTls {
        LeaderTls {
            client_config: crate::tls::build_client_config(Some(ca), false).unwrap(),
            server_name: rustls::pki_types::ServerName::try_from("127.0.0.1").unwrap(),
        }
    }

    #[tokio::test]
    async fn replication_over_tls() {
        use crate::tls::test_support::*;
        let cert = TempPem::new("repl_cert", TEST_SERVER_CERT_PEM);
        let key = TempPem::new("repl_key", TEST_SERVER_KEY_PEM);
        let other_ca = TempPem::new("repl_other_ca", TEST_SERVER_CERT2_PEM);
        let acceptor =
            ReloadableTlsAcceptor::new(crate::tls::TlsConfig::new(cert.path(), key.path()))
                .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let leader = Arc::new(
            LeaderReplicationManager::new(tls_config(NodeRole::Leader, &addr.to_string(), None))
                .with_tls(acceptor),
        );
        leader.start_with_listener(listener).await.unwrap();

        let new_follower = |tls: Option<LeaderTls>| {
            let graph = Arc::new(ConcurrentGraph::new());
            let mut f = FollowerReplicationManager::with_concurrent_graph(
                tls_config(NodeRole::Follower, "127.0.0.1:0", Some(addr.to_string())),
                Arc::clone(&graph),
            );
            if let Some(t) = tls {
                f = f.with_leader_tls(t);
            }
            (Arc::new(f), graph)
        };

        // A follower that trusts the leader's certificate replicates.
        let (follower, graph) = new_follower(Some(leader_tls(cert.path())));
        follower.start().await.unwrap();
        // A plaintext follower never completes the handshake.
        let (plain, _) = new_follower(None);
        plain.start().await.unwrap();
        // A follower trusting another certificate cannot connect.
        let (untrusting, _) = new_follower(Some(leader_tls(other_ca.path())));
        assert!(untrusting.start().await.is_err());

        for _ in 0..100 {
            if leader.get_stats().follower_count == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        leader
            .append_wal_entry(WalEntryData::CreateNode {
                node_id: 0,
                labels: vec!["Secret".to_string()],
            })
            .await;
        let mut replicated = false;
        for _ in 0..100 {
            if graph.node_count() == 1 {
                replicated = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(replicated, "WAL entry did not reach the TLS follower");
        assert!(follower.is_leader_alive());
        assert!(!plain.is_leader_alive());
        assert_eq!(leader.get_stats().follower_count, 1);
    }
}
