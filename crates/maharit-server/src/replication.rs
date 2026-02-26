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
    /// A node was created
    CreateNode { node_id: u64, label: String },
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
    /// Follower -> Leader: initiate a replication session
    Handshake { follower_id: String },
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

        let config = self.config.clone();
        let lsn = Arc::clone(&self.lsn);
        let followers = Arc::clone(&self.followers);
        let shutdown = Arc::clone(&self.shutdown);
        let wal_sender = self.wal_sender.clone();

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

                        tokio::spawn(async move {
                            if let Err(e) = handle_follower_connection(
                                socket, config2, lsn2, followers2, shutdown2, wal_rx,
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

/// Handle a single follower connection on the leader side.
async fn handle_follower_connection(
    mut socket: TcpStream,
    config: ReplicationConfig,
    lsn: Arc<AtomicU64>,
    followers: Arc<RwLock<HashMap<String, FollowerState>>>,
    shutdown: Arc<AtomicBool>,
    mut wal_rx: broadcast::Receiver<(u64, WalEntryData)>,
) -> Result<(), ReplicationError> {
    let (mut reader, mut writer) = socket.split();

    // ── Handshake ────────────────────────────────────────────────────────────
    let msg = recv_message(&mut reader).await?;
    let follower_id = match msg {
        ReplicationMessage::Handshake { follower_id } => follower_id,
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

    // Register the follower.
    {
        let mut guard = followers.write().await;
        guard.insert(
            follower_id.clone(),
            FollowerState {
                follower_id: follower_id.clone(),
                last_lsn: 0,
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
        }
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

        // ── Receive loop ─────────────────────────────────────────────────────
        tokio::spawn(async move {
            if let Err(e) = run_follower_receive_loop(
                stream,
                config,
                current_lsn,
                is_leader_alive,
                last_heartbeat,
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

/// Core receive loop executed in a background task on the follower.
async fn run_follower_receive_loop(
    mut stream: TcpStream,
    config: ReplicationConfig,
    current_lsn: Arc<AtomicU64>,
    is_leader_alive: Arc<AtomicBool>,
    last_heartbeat: Arc<RwLock<Instant>>,
) -> Result<(), ReplicationError> {
    let (mut reader, mut writer) = stream.split();

    // ── Handshake ────────────────────────────────────────────────────────────
    send_message(
        &mut writer,
        &ReplicationMessage::Handshake {
            follower_id: config.node_id.clone(),
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

            ReplicationMessage::WalEntry { lsn, entry: _ } => {
                // In a full implementation the entry would be applied to the
                // local graph here.  For now we just advance the LSN counter.
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
            label: "Person".to_string(),
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
                    label: "Test".to_string(),
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
                label: "Person".to_string(),
            })
            .await;
        assert_eq!(lsn1, 1);

        let lsn2 = manager
            .append_wal_entry(WalEntryData::CreateNode {
                node_id: 2,
                label: "City".to_string(),
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
}
