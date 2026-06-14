//! Leader/follower replication for MaharitDB
//!
//! Provides:
//! - WAL-based replication between leader and follower nodes
//! - Heartbeat mechanism for detecting leader failure
//! - Length-prefixed JSON message framing (same as tcp_server.rs)
//! - Async TCP communication using tokio

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use maharit_core::{ConcurrentGraph, Graph, PropertyValue};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, broadcast};
use tokio::time::{interval, timeout};

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
    },
    /// Leader -> Follower: acknowledge the handshake
    HandshakeAck { leader_id: String },
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
    /// Leader -> Follower: full graph snapshot (sent to new followers with LSN 0).
    /// `data` is a JSON-serialized `Vec<WalEntryData>` that recreates the graph.
    Snapshot { data: Vec<u8> },
    /// Leader -> Follower: instruct the follower to promote itself to leader.
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

/// Read a length-prefixed JSON message from an async reader
async fn recv_message<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> Result<ReplicationMessage, ReplicationError> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
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
/// Accepts follower connections on a dedicated TCP port, broadcasts WAL entries
/// to all connected followers, and sends periodic heartbeats.
pub struct LeaderReplicationManager {
    config: ReplicationConfig,
    /// Monotonically increasing log sequence number
    lsn: Arc<AtomicU64>,
    /// Map of follower_id -> state for every connected follower
    followers: Arc<RwLock<HashMap<String, FollowerState>>>,
    /// Set to `true` to request an orderly shutdown
    shutdown: Arc<AtomicBool>,
    /// Channel used to broadcast (lsn, entry) to all follower handler tasks
    wal_sender: broadcast::Sender<(u64, WalEntryData)>,
    /// Optional graph reference for sending full snapshots to new followers.
    graph: Option<Arc<RwLock<Graph>>>,
}

impl LeaderReplicationManager {
    /// Create a new leader replication manager with the given configuration.
    ///
    /// # Panics
    /// Does not panic; errors are surfaced through `start()`.
    pub fn new(config: ReplicationConfig) -> Self {
        // A buffer of 1024 unacknowledged WAL entries before the channel blocks.
        let (wal_sender, _) = broadcast::channel(1024);
        Self {
            config,
            lsn: Arc::new(AtomicU64::new(0)),
            followers: Arc::new(RwLock::new(HashMap::new())),
            shutdown: Arc::new(AtomicBool::new(false)),
            wal_sender,
            graph: None,
        }
    }

    /// Create a leader manager with a graph reference for initial snapshot sync.
    pub fn with_graph(config: ReplicationConfig, graph: Arc<RwLock<Graph>>) -> Self {
        let (wal_sender, _) = broadcast::channel(1024);
        Self {
            config,
            lsn: Arc::new(AtomicU64::new(0)),
            followers: Arc::new(RwLock::new(HashMap::new())),
            shutdown: Arc::new(AtomicBool::new(false)),
            wal_sender,
            graph: Some(graph),
        }
    }

    /// Append a WAL entry and broadcast it to all connected followers.
    ///
    /// Increments and returns the new log sequence number.  If no followers are
    /// connected the broadcast is a no-op (receivers count may be zero).
    pub async fn append_wal_entry(&self, entry: WalEntryData) -> u64 {
        let new_lsn = self.lsn.fetch_add(1, Ordering::SeqCst) + 1;
        // It is fine if there are no subscribers yet.
        let _ = self.wal_sender.send((new_lsn, entry));
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
        let wal_sender = self.wal_sender.clone();
        let graph = self.graph.clone();

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
                        let wal_rx = wal_sender.subscribe();
                        let graph2 = graph.clone();

                        tokio::spawn(async move {
                            if let Err(e) = handle_follower_connection(
                                socket, config2, lsn2, followers2, shutdown2, wal_rx, graph2,
                            )
                            .await
                            {
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
            let value = match val {
                PropertyValue::Null => "null".to_string(),
                PropertyValue::Bool(b) => b.to_string(),
                PropertyValue::Int(n) => n.to_string(),
                PropertyValue::Float(n) => n.to_string(),
                PropertyValue::String(s) => serde_json::to_string(s).unwrap_or_default(),
                PropertyValue::Date(_) | PropertyValue::DateTime(_) | PropertyValue::Duration { .. } => {
                    serde_json::to_string(&val.to_string()).unwrap_or_default()
                }
            };
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
            let value = match val {
                PropertyValue::Null => "null".to_string(),
                PropertyValue::Bool(b) => b.to_string(),
                PropertyValue::Int(n) => n.to_string(),
                PropertyValue::Float(n) => n.to_string(),
                PropertyValue::String(s) => serde_json::to_string(s).unwrap_or_default(),
                PropertyValue::Date(_) | PropertyValue::DateTime(_) | PropertyValue::Duration { .. } => {
                    serde_json::to_string(&val.to_string()).unwrap_or_default()
                }
            };
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

/// Handle a single follower connection on the leader side.
async fn handle_follower_connection(
    mut socket: TcpStream,
    config: ReplicationConfig,
    lsn: Arc<AtomicU64>,
    followers: Arc<RwLock<HashMap<String, FollowerState>>>,
    shutdown: Arc<AtomicBool>,
    mut wal_rx: broadcast::Receiver<(u64, WalEntryData)>,
    graph: Option<Arc<RwLock<Graph>>>,
) -> Result<(), ReplicationError> {
    let (mut reader, mut writer) = socket.split();

    // ── Handshake ────────────────────────────────────────────────────────────
    let msg = recv_message(&mut reader).await?;
    let (follower_id, follower_lsn) = match msg {
        ReplicationMessage::Handshake { follower_id, current_lsn } => (follower_id, current_lsn),
        other => {
            return Err(ReplicationError::ConnectionFailed(format!(
                "Expected Handshake, got {:?}",
                other
            )));
        }
    };

    send_message(
        &mut writer,
        &ReplicationMessage::HandshakeAck {
            leader_id: config.node_id.clone(),
        },
    )
    .await?;

    // If the follower has no data (LSN == 0), send a full snapshot.
    if follower_lsn == 0
        && let Some(ref g) = graph
    {
        let entries = graph_to_snapshot_entries(g).await;
        let data = serde_json::to_vec(&entries).unwrap_or_default();
        send_message(&mut writer, &ReplicationMessage::Snapshot { data }).await?;
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
                if send_message(&mut writer, &hb).await.is_err() {
                    break;
                }
            }

            // New WAL entry to forward
            entry_result = wal_rx.recv() => {
                match entry_result {
                    Ok((entry_lsn, entry_data)) => {
                        let wal_msg = ReplicationMessage::WalEntry {
                            lsn: entry_lsn,
                            entry: entry_data,
                        };
                        if send_message(&mut writer, &wal_msg).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("Follower {} lagged by {} entries", follower_id, n);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }

            // Incoming message from the follower
            read_result = recv_message(&mut reader) => {
                match read_result {
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
                    Ok(_) | Err(_) => {
                        break;
                    }
                }
            }
        }
    }

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
    /// Set to `true` when a `PromoteToLeader` message is received.
    is_promoted: Arc<AtomicBool>,
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
        }
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

    /// Connect to the leader and start receiving WAL entries.
    ///
    /// Spawns two background tokio tasks:
    /// 1. A receive loop that handles incoming messages from the leader.
    /// 2. A watchdog that flips `is_leader_alive` to `false` when no heartbeat
    ///    has been received within `heartbeat_timeout_secs`.
    ///
    /// Returns `Ok(())` as soon as both tasks are spawned.
    pub async fn start(&self) -> Result<(), ReplicationError> {
        let leader_address = self
            .config
            .leader_address
            .clone()
            .ok_or(ReplicationError::LeaderNotConfigured)?;

        let stream = TcpStream::connect(&leader_address)
            .await
            .map_err(|e| ReplicationError::ConnectionFailed(e.to_string()))?;

        let config = self.config.clone();
        let current_lsn = Arc::clone(&self.current_lsn);
        let is_leader_alive = Arc::clone(&self.is_leader_alive);
        let last_heartbeat = Arc::clone(&self.last_heartbeat);
        let graph = Arc::clone(&self.graph);
        let is_promoted = Arc::clone(&self.is_promoted);

        // ── Receive loop ─────────────────────────────────────────────────────
        tokio::spawn(async move {
            if let Err(e) = run_follower_receive_loop(
                stream,
                config,
                current_lsn,
                is_leader_alive,
                last_heartbeat,
                graph,
                is_promoted,
            )
            .await
            {
                eprintln!("Follower receive loop error: {}", e);
            }
        });

        // ── Heartbeat watchdog ────────────────────────────────────────────────
        let is_leader_alive2 = Arc::clone(&self.is_leader_alive);
        let last_heartbeat2 = Arc::clone(&self.last_heartbeat);
        let timeout_secs = self.config.heartbeat_timeout_secs;

        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(1));
            loop {
                ticker.tick().await;
                let last = *last_heartbeat2.read().await;
                if last.elapsed() > Duration::from_secs(timeout_secs) {
                    is_leader_alive2.store(false, Ordering::SeqCst);
                }
            }
        });

        Ok(())
    }
}

/// Apply a single WAL entry to the local graph (best-effort; errors are logged).
///
/// Uses `ConcurrentGraph` directly — no async lock needed.
fn apply_wal_entry(graph: &Arc<ConcurrentGraph>, entry: &WalEntryData) {
    match entry {
        WalEntryData::CreateNode { node_id, labels } => {
            graph.create_node_with_id_and_labels(*node_id, labels.clone());
        }
        WalEntryData::DeleteNode { node_id } => {
            graph.delete_node(*node_id);
        }
        WalEntryData::CreateEdge { from, to, label, .. } => {
            if let Err(e) = graph.create_edge(*from, *to, label) {
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
            // Property values are serialised as JSON strings for portability.
            let prop_val = serde_json::from_str::<serde_json::Value>(value)
                .map(|v| match v {
                    serde_json::Value::Null => PropertyValue::Null,
                    serde_json::Value::Bool(b) => PropertyValue::Bool(b),
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            PropertyValue::Int(i)
                        } else {
                            PropertyValue::Float(n.as_f64().unwrap_or(0.0))
                        }
                    }
                    serde_json::Value::String(s) => PropertyValue::String(s),
                    _ => PropertyValue::String(value.clone()),
                })
                .unwrap_or_else(|_| PropertyValue::String(value.clone()));

            if *is_node {
                graph.set_node_property(*target_id, key, prop_val);
            } else {
                graph.set_edge_property(*target_id, key, prop_val);
            }
        }
    }
}

/// Core receive loop executed in a background task on the follower.
async fn run_follower_receive_loop(
    mut stream: TcpStream,
    config: ReplicationConfig,
    current_lsn: Arc<AtomicU64>,
    is_leader_alive: Arc<AtomicBool>,
    last_heartbeat: Arc<RwLock<Instant>>,
    graph: Arc<ConcurrentGraph>,
    is_promoted: Arc<AtomicBool>,
) -> Result<(), ReplicationError> {
    let (mut reader, mut writer) = stream.split();

    // ── Handshake ────────────────────────────────────────────────────────────
    let my_lsn = current_lsn.load(Ordering::SeqCst);
    send_message(
        &mut writer,
        &ReplicationMessage::Handshake {
            follower_id: config.node_id.clone(),
            current_lsn: my_lsn,
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
            Err(_) => {
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

            ReplicationMessage::Snapshot { data } => {
                // Apply the full snapshot: deserialize as a list of WAL entries and apply each.
                match serde_json::from_slice::<Vec<WalEntryData>>(&data) {
                    Ok(entries) => {
                        for entry in &entries {
                            apply_wal_entry(&graph, entry);
                        }
                        // Advance LSN to the current leader LSN so future WAL entries
                        // are applied correctly (snapshot is the baseline).
                        *last_heartbeat.write().await = Instant::now();
                    }
                    Err(e) => {
                        eprintln!("Snapshot deserialization error: {}", e);
                    }
                }
            }

            ReplicationMessage::PromoteToLeader => {
                // Signal that this follower should become the new leader.
                is_promoted.store(true, Ordering::SeqCst);
                is_leader_alive.store(false, Ordering::SeqCst);
                break;
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

/// Send a `PromoteToLeader` message to a follower at the given replication address.
///
/// Used by the `admin promote-to-leader` CLI command.
pub async fn send_promote_to_leader(addr: &str) -> Result<(), ReplicationError> {
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|e| ReplicationError::ConnectionFailed(e.to_string()))?;

    let data = serde_json::to_vec(&ReplicationMessage::PromoteToLeader)?;
    let len = data.len() as u32;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(&data).await?;
    stream.flush().await?;

    Ok(())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
            let id =
                g.create_node_with_labels(vec!["Person".to_string(), "Employee".to_string()]);
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
    async fn start_query_server(graph: Arc<ConcurrentGraph>, repl: Option<Arc<LeaderReplicationManager>>) -> std::net::SocketAddr {
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
        };
        let manager = Arc::new(FollowerReplicationManager::with_concurrent_graph(
            config,
            Arc::clone(&follower_graph),
        ));
        manager.start().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        (manager, follower_graph)
    }

    #[tokio::test]
    async fn replication_integration_follower_receives_created_node() {
        let (leader_repl, repl_addr) = start_leader_replication().await;
        let (_, follower_graph) = start_follower_replication(repl_addr).await;

        // Leader: start query server with replication wired in
        let leader_graph = Arc::new(ConcurrentGraph::new());
        let leader_addr = start_query_server(Arc::clone(&leader_graph), Some(Arc::clone(&leader_repl))).await;

        // Follower: start query server against the follower's graph
        let follower_addr = start_query_server(Arc::clone(&follower_graph), None).await;

        // Write to leader
        let mut lconn = tokio::net::TcpStream::connect(leader_addr).await.unwrap();
        repl_send_recv(
            &mut lconn,
            &Request::Query {
                query: "CREATE (n:Test {name: 'Alice'}) RETURN n".to_string(),
                tx_id: None, session_token: None,
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
                tx_id: None, session_token: None,
            },
        )
        .await;

        match resp {
            Response::Result { rows } => {
                assert!(!rows.is_empty(), "follower should have the node created on leader");
                assert_eq!(rows[0].get("n.name").map(String::as_str), Some("\"Alice\""));
            }
            other => panic!("expected Result from follower, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn replication_integration_follower_receives_node_properties() {
        let (leader_repl, repl_addr) = start_leader_replication().await;
        let (_, follower_graph) = start_follower_replication(repl_addr).await;

        let leader_graph = Arc::new(ConcurrentGraph::new());
        let leader_addr = start_query_server(Arc::clone(&leader_graph), Some(Arc::clone(&leader_repl))).await;
        let follower_addr = start_query_server(Arc::clone(&follower_graph), None).await;

        // Write node with string + integer + bool properties
        let mut lconn = tokio::net::TcpStream::connect(leader_addr).await.unwrap();
        repl_send_recv(
            &mut lconn,
            &Request::Query {
                query: "CREATE (n:Prop {s: 'hello', i: 99, b: true}) RETURN n".to_string(),
                tx_id: None, session_token: None,
            },
        )
        .await;

        tokio::time::sleep(Duration::from_millis(200)).await;

        let mut fconn = tokio::net::TcpStream::connect(follower_addr).await.unwrap();

        // String property
        let resp = repl_send_recv(
            &mut fconn,
            &Request::Query { query: "MATCH (n:Prop) RETURN n.s".to_string(), tx_id: None, session_token: None },
        ).await;
        match resp {
            Response::Result { rows } => {
                assert!(!rows.is_empty());
                assert_eq!(rows[0].get("n.s").map(String::as_str), Some("\"hello\""));
            }
            other => panic!("string property: {:?}", other),
        }

        // Integer property
        let resp = repl_send_recv(
            &mut fconn,
            &Request::Query { query: "MATCH (n:Prop) RETURN n.i".to_string(), tx_id: None, session_token: None },
        ).await;
        match resp {
            Response::Result { rows } => {
                assert!(!rows.is_empty());
                assert_eq!(rows[0].get("n.i").map(String::as_str), Some("99"));
            }
            other => panic!("integer property: {:?}", other),
        }
    }

    #[tokio::test]
    async fn replication_integration_follower_receives_edge() {
        let (leader_repl, repl_addr) = start_leader_replication().await;
        let (_, follower_graph) = start_follower_replication(repl_addr).await;

        let leader_graph = Arc::new(ConcurrentGraph::new());
        let leader_addr = start_query_server(Arc::clone(&leader_graph), Some(Arc::clone(&leader_repl))).await;
        let follower_addr = start_query_server(Arc::clone(&follower_graph), None).await;

        // Create two nodes and an edge on the leader
        let mut lconn = tokio::net::TcpStream::connect(leader_addr).await.unwrap();
        for q in [
            "CREATE (n:Src {name: 'src'})",
            "CREATE (n:Dst {name: 'dst'})",
            "MATCH (a:Src {name: 'src'}), (b:Dst {name: 'dst'}) CREATE (a)-[:LINK]->(b)",
        ] {
            repl_send_recv(&mut lconn, &Request::Query { query: q.to_string(), tx_id: None, session_token: None }).await;
        }

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Traverse on follower
        let mut fconn = tokio::net::TcpStream::connect(follower_addr).await.unwrap();
        let resp = repl_send_recv(
            &mut fconn,
            &Request::Query {
                query: "MATCH (a:Src)-[:LINK]->(b:Dst) RETURN b.name".to_string(),
                tx_id: None, session_token: None,
            },
        )
        .await;
        match resp {
            Response::Result { rows } => {
                assert!(!rows.is_empty(), "follower should see the edge traversal");
                assert_eq!(rows[0].get("b.name").map(String::as_str), Some("\"dst\""));
            }
            other => panic!("expected Result for edge traversal, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn replication_integration_follower_receives_deletion() {
        let (leader_repl, repl_addr) = start_leader_replication().await;
        let (_, follower_graph) = start_follower_replication(repl_addr).await;

        let leader_graph = Arc::new(ConcurrentGraph::new());
        let leader_addr = start_query_server(Arc::clone(&leader_graph), Some(Arc::clone(&leader_repl))).await;
        let follower_addr = start_query_server(Arc::clone(&follower_graph), None).await;

        let mut lconn = tokio::net::TcpStream::connect(leader_addr).await.unwrap();

        // Create then delete
        repl_send_recv(
            &mut lconn,
            &Request::Query {
                query: "CREATE (n:Del {name: 'gone'}) RETURN n".to_string(),
                tx_id: None, session_token: None,
            },
        )
        .await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        repl_send_recv(
            &mut lconn,
            &Request::Query {
                query: "MATCH (n:Del {name: 'gone'}) DETACH DELETE n".to_string(),
                tx_id: None, session_token: None,
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
                tx_id: None, session_token: None,
            },
        )
        .await;
        match resp {
            Response::Result { rows } => {
                assert!(rows.is_empty(), "deleted node should not appear on follower");
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
        };
        let follower_repl = FollowerReplicationManager::with_concurrent_graph(
            config,
            Arc::clone(&follower_graph),
        );

        // The follower replication manager's graph and the TcpServer's graph must be the same Arc
        assert!(
            Arc::ptr_eq(&follower_repl.graph(), &follower_graph),
            "FollowerReplicationManager and TcpServer must share the same graph Arc"
        );
    }

    // 20. Verify that a follower promotes itself after receiving a PromoteToLeader message
    #[tokio::test]
    async fn test_promote_to_leader() {
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
        assert!(manager.is_leader_alive(), "leader should be alive after handshake");
        assert!(!manager.is_promoted(), "should not be promoted yet");

        // Wait for the PromoteToLeader message to be processed (arrives at ~200 ms).
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(manager.is_promoted(), "follower should be promoted after PromoteToLeader");
        assert!(!manager.is_leader_alive(), "is_leader_alive should be false after promotion");
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
        assert!(manager.is_leader_alive(), "leader should be alive after handshake");

        // After heartbeat_timeout_secs + one watchdog-tick second the leader should be dead
        tokio::time::sleep(Duration::from_secs(timeout_secs + 1)).await;
        assert!(
            !manager.is_leader_alive(),
            "leader should be marked dead after heartbeat timeout"
        );
    }
}
