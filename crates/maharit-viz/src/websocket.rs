//! WebSocket server for real-time graph change broadcasting.
//!
//! This module provides a WebSocket server that pushes [`GraphEvent`] messages
//! to every connected browser client.  The server uses a
//! [`tokio::sync::broadcast`] channel internally so that events produced on any
//! thread are fanned out to all active connections.
//!
//! # Quick start
//!
//! ```no_run
//! use maharit_viz::websocket::{GraphWebSocketServer, GraphEvent};
//!
//! # async fn example() {
//! let server = GraphWebSocketServer::new("127.0.0.1:8765");
//! let tx = server.event_sender();
//!
//! // Spawn the server in the background.
//! tokio::spawn(server.start());
//!
//! // Notify clients of a new node.
//! let _ = tx.send(GraphEvent::NodeCreated {
//!     node_id: 1,
//!     label: "Person".to_string(),
//!     properties: Default::default(),
//! });
//! # }
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, broadcast};
use tokio_tungstenite::tungstenite::Message;

// ---------------------------------------------------------------------------
// Public data types
// ---------------------------------------------------------------------------

/// A JSON-serialisable representation of a graph node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeJson {
    /// Unique node identifier.
    pub id: u64,
    /// Node label (type).
    pub label: String,
    /// Arbitrary key/value properties.
    pub properties: HashMap<String, serde_json::Value>,
}

/// A JSON-serialisable representation of a graph edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeJson {
    /// Unique edge identifier.
    pub id: u64,
    /// Source node id.
    pub from: u64,
    /// Destination node id.
    pub to: u64,
    /// Edge label (relationship type).
    pub label: String,
}

/// Events emitted whenever the graph changes.
///
/// Each variant is serialised to JSON and pushed to every connected WebSocket
/// client.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum GraphEvent {
    /// A new node was created.
    NodeCreated {
        node_id: u64,
        label: String,
        properties: HashMap<String, serde_json::Value>,
    },
    /// A node was deleted.
    NodeDeleted { node_id: u64 },
    /// Node properties were updated.
    NodeUpdated {
        node_id: u64,
        properties: HashMap<String, serde_json::Value>,
    },
    /// A new edge was created.
    EdgeCreated {
        edge_id: u64,
        from: u64,
        to: u64,
        label: String,
    },
    /// An edge was deleted.
    EdgeDeleted { edge_id: u64 },
    /// Complete graph snapshot sent immediately after a client connects or on
    /// explicit request.
    FullSnapshot {
        nodes: Vec<NodeJson>,
        edges: Vec<EdgeJson>,
    },
}

/// Messages that a browser client can send to the server.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Execute a Cypher-like query.
    RunQuery { query: String },
    /// Request the server to re-send a full graph snapshot.
    RequestSnapshot,
}

// ---------------------------------------------------------------------------
// WebSocket server
// ---------------------------------------------------------------------------

/// Capacity of the broadcast channel (number of buffered events before the
/// oldest is dropped for slow consumers).
const BROADCAST_CAPACITY: usize = 256;

/// Shared initial snapshot state sent to newly connected clients.
type SnapshotState = Arc<Mutex<Option<(Vec<NodeJson>, Vec<EdgeJson>)>>>;

/// WebSocket server that broadcasts [`GraphEvent`]s to every connected client.
///
/// Use [`GraphWebSocketServer::event_sender`] to obtain a
/// [`broadcast::Sender<GraphEvent>`] that can be cloned and moved to any task
/// that produces graph changes.
pub struct GraphWebSocketServer {
    addr: String,
    event_tx: broadcast::Sender<GraphEvent>,
    /// Optional shared snapshot provider sent to newly connected clients.
    snapshot: SnapshotState,
}

impl GraphWebSocketServer {
    /// Create a new server bound to the given address (e.g. `"127.0.0.1:8765"`).
    pub fn new(addr: &str) -> Self {
        let (event_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            addr: addr.to_owned(),
            event_tx,
            snapshot: Arc::new(Mutex::new(None)),
        }
    }

    /// Return a sender that can be used to push events to all connected clients.
    ///
    /// The returned sender can be cloned freely and sent across threads.
    pub fn event_sender(&self) -> broadcast::Sender<GraphEvent> {
        self.event_tx.clone()
    }

    /// Replace the cached snapshot that is sent to newly connected clients.
    pub async fn set_snapshot(&self, nodes: Vec<NodeJson>, edges: Vec<EdgeJson>) {
        let mut guard = self.snapshot.lock().await;
        *guard = Some((nodes, edges));
    }

    /// Start accepting WebSocket connections.
    ///
    /// This method consumes `self` and runs indefinitely.  Spawn it with
    /// `tokio::spawn` so it runs concurrently with the rest of the application.
    pub async fn start(self) {
        let listener = match TcpListener::bind(&self.addr).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[websocket] Failed to bind {}: {}", self.addr, e);
                return;
            }
        };

        eprintln!("[websocket] Listening on ws://{}", self.addr);

        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    let event_rx = self.event_tx.subscribe();
                    let snapshot = Arc::clone(&self.snapshot);
                    tokio::spawn(async move {
                        Self::handle_connection(stream, event_rx, snapshot, peer_addr).await;
                    });
                }
                Err(e) => {
                    eprintln!("[websocket] Accept error: {}", e);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Handle a single WebSocket client connection.
    ///
    /// 1. Performs the HTTP upgrade handshake.
    /// 2. Optionally sends a `FullSnapshot` if one is cached.
    /// 3. Forwards every incoming broadcast event to the client as a JSON
    ///    text frame.
    /// 4. Reads incoming text frames from the client and interprets them as
    ///    [`ClientMessage`] values.
    async fn handle_connection(
        stream: TcpStream,
        mut event_rx: broadcast::Receiver<GraphEvent>,
        snapshot: SnapshotState,
        peer_addr: std::net::SocketAddr,
    ) {
        let ws_stream = match tokio_tungstenite::accept_async(stream).await {
            Ok(ws) => ws,
            Err(e) => {
                eprintln!("[websocket] Handshake failed for {}: {}", peer_addr, e);
                return;
            }
        };

        let (mut ws_sink, mut ws_source) = ws_stream.split();

        // Send FullSnapshot immediately after connecting.
        {
            let guard = snapshot.lock().await;
            if let Some((nodes, edges)) = guard.as_ref() {
                let evt = GraphEvent::FullSnapshot {
                    nodes: nodes.clone(),
                    edges: edges.clone(),
                };
                if let Ok(json) = serde_json::to_string(&evt) {
                    let _ = ws_sink.send(Message::Text(json.into())).await;
                }
            }
        }

        // Drive the send/receive loop.
        loop {
            tokio::select! {
                // Broadcast channel -> client
                result = event_rx.recv() => {
                    match result {
                        Ok(event) => {
                            match serde_json::to_string(&event) {
                                Ok(json) => {
                                    if ws_sink.send(Message::Text(json.into())).await.is_err() {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[websocket] Serialise error: {}", e);
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            eprintln!(
                                "[websocket] Client {} lagged by {} events",
                                peer_addr, n
                            );
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }

                // Client -> server messages
                msg = ws_source.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            Self::process_client_message(&text, peer_addr);
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Err(e)) => {
                            use tokio_tungstenite::tungstenite::Error as WsError;
                            match e {
                                WsError::ConnectionClosed
                                | WsError::AlreadyClosed => break,
                                other => {
                                    eprintln!(
                                        "[websocket] Read error from {}: {}",
                                        peer_addr, other
                                    );
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        eprintln!("[websocket] Client {} disconnected", peer_addr);
    }

    /// Parse and handle a text frame sent by a browser client.
    fn process_client_message(text: &str, peer_addr: std::net::SocketAddr) {
        match serde_json::from_str::<ClientMessage>(text) {
            Ok(ClientMessage::RunQuery { ref query }) => {
                eprintln!("[websocket] RunQuery from {}: {}", peer_addr, query);
                // Full query execution would be wired via Arc<Mutex<Executor>>.
            }
            Ok(ClientMessage::RequestSnapshot) => {
                eprintln!("[websocket] RequestSnapshot from {}", peer_addr);
            }
            Err(e) => {
                eprintln!(
                    "[websocket] Unrecognised message from {}: {} ({:?})",
                    peer_addr, text, e
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // GraphWebSocketServer construction
    // ------------------------------------------------------------------

    #[test]
    fn test_server_new_stores_addr() {
        let server = GraphWebSocketServer::new("127.0.0.1:8765");
        assert_eq!(server.addr, "127.0.0.1:8765");
    }

    #[test]
    fn test_event_sender_returns_sender() {
        let server = GraphWebSocketServer::new("127.0.0.1:8766");
        let tx = server.event_sender();
        // Subscribing proves the sender is valid.
        let _rx = tx.subscribe();
    }

    #[test]
    fn test_event_sender_clone_and_subscribe() {
        let server = GraphWebSocketServer::new("127.0.0.1:8767");
        let tx = server.event_sender();
        let mut rx = tx.subscribe();

        let tx2 = tx.clone();
        tx2.send(GraphEvent::EdgeDeleted { edge_id: 7 }).unwrap();

        let received = rx.try_recv().expect("should receive event");
        assert!(matches!(received, GraphEvent::EdgeDeleted { edge_id: 7 }));
    }

    // ------------------------------------------------------------------
    // GraphEvent serialisation
    // ------------------------------------------------------------------

    #[test]
    fn test_node_created_serialises() {
        let evt = GraphEvent::NodeCreated {
            node_id: 1,
            label: "Person".to_string(),
            properties: {
                let mut m = HashMap::new();
                m.insert("name".to_string(), serde_json::json!("Alice"));
                m
            },
        };
        let json = serde_json::to_string(&evt).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["event"], "node_created");
        assert_eq!(v["node_id"], 1u64);
        assert_eq!(v["label"], "Person");
        assert_eq!(v["properties"]["name"], "Alice");
    }

    #[test]
    fn test_node_deleted_serialises() {
        let evt = GraphEvent::NodeDeleted { node_id: 99 };
        let json = serde_json::to_string(&evt).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["event"], "node_deleted");
        assert_eq!(v["node_id"], 99u64);
    }

    #[test]
    fn test_node_updated_serialises() {
        let evt = GraphEvent::NodeUpdated {
            node_id: 3,
            properties: {
                let mut m = HashMap::new();
                m.insert("age".to_string(), serde_json::json!(30));
                m
            },
        };
        let json = serde_json::to_string(&evt).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["event"], "node_updated");
        assert_eq!(v["node_id"], 3u64);
        assert_eq!(v["properties"]["age"], 30);
    }

    #[test]
    fn test_edge_created_serialises() {
        let evt = GraphEvent::EdgeCreated {
            edge_id: 10,
            from: 1,
            to: 2,
            label: "KNOWS".to_string(),
        };
        let json = serde_json::to_string(&evt).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["event"], "edge_created");
        assert_eq!(v["edge_id"], 10u64);
        assert_eq!(v["from"], 1u64);
        assert_eq!(v["to"], 2u64);
        assert_eq!(v["label"], "KNOWS");
    }

    #[test]
    fn test_edge_deleted_serialises() {
        let evt = GraphEvent::EdgeDeleted { edge_id: 5 };
        let json = serde_json::to_string(&evt).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["event"], "edge_deleted");
        assert_eq!(v["edge_id"], 5u64);
    }

    #[test]
    fn test_full_snapshot_serialises_correctly() {
        let nodes = vec![NodeJson {
            id: 1,
            label: "Person".to_string(),
            properties: {
                let mut m = HashMap::new();
                m.insert("name".to_string(), serde_json::json!("Bob"));
                m
            },
        }];
        let edges = vec![EdgeJson {
            id: 0,
            from: 1,
            to: 2,
            label: "KNOWS".to_string(),
        }];

        let evt = GraphEvent::FullSnapshot {
            nodes: nodes.clone(),
            edges: edges.clone(),
        };
        let json = serde_json::to_string(&evt).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(v["event"], "full_snapshot");
        assert!(v["nodes"].is_array());
        assert_eq!(v["nodes"][0]["id"], 1u64);
        assert_eq!(v["nodes"][0]["label"], "Person");
        assert_eq!(v["nodes"][0]["properties"]["name"], "Bob");
        assert!(v["edges"].is_array());
        assert_eq!(v["edges"][0]["label"], "KNOWS");
        assert_eq!(v["edges"][0]["from"], 1u64);
        assert_eq!(v["edges"][0]["to"], 2u64);
    }

    // ------------------------------------------------------------------
    // ClientMessage deserialisation
    // ------------------------------------------------------------------

    #[test]
    fn test_client_message_run_query_deserialises() {
        let json = r#"{"action":"run_query","query":"MATCH (n) RETURN n"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::RunQuery { query } => {
                assert_eq!(query, "MATCH (n) RETURN n");
            }
            other => panic!("unexpected variant: {:?}", other),
        }
    }

    #[test]
    fn test_client_message_request_snapshot_deserialises() {
        let json = r#"{"action":"request_snapshot"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::RequestSnapshot));
    }

    // ------------------------------------------------------------------
    // NodeJson / EdgeJson round-trip
    // ------------------------------------------------------------------

    #[test]
    fn test_node_json_round_trip() {
        let node = NodeJson {
            id: 42,
            label: "City".to_string(),
            properties: {
                let mut m = HashMap::new();
                m.insert("population".to_string(), serde_json::json!(3_500_000));
                m
            },
        };
        let json = serde_json::to_string(&node).unwrap();
        let decoded: NodeJson = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, 42);
        assert_eq!(decoded.label, "City");
        assert_eq!(decoded.properties["population"], 3_500_000);
    }

    #[test]
    fn test_edge_json_round_trip() {
        let edge = EdgeJson {
            id: 7,
            from: 1,
            to: 2,
            label: "LIVES_IN".to_string(),
        };
        let json = serde_json::to_string(&edge).unwrap();
        let decoded: EdgeJson = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, 7);
        assert_eq!(decoded.from, 1);
        assert_eq!(decoded.to, 2);
        assert_eq!(decoded.label, "LIVES_IN");
    }

    // ------------------------------------------------------------------
    // Broadcast channel behaviour
    // ------------------------------------------------------------------

    #[test]
    fn test_broadcast_multiple_receivers() {
        let server = GraphWebSocketServer::new("127.0.0.1:8768");
        let tx = server.event_sender();

        let mut rx1 = tx.subscribe();
        let mut rx2 = tx.subscribe();

        tx.send(GraphEvent::NodeDeleted { node_id: 1 }).unwrap();

        let ev1 = rx1.try_recv().unwrap();
        let ev2 = rx2.try_recv().unwrap();

        assert!(matches!(ev1, GraphEvent::NodeDeleted { node_id: 1 }));
        assert!(matches!(ev2, GraphEvent::NodeDeleted { node_id: 1 }));
    }

    #[test]
    fn test_empty_full_snapshot_serialises() {
        let evt = GraphEvent::FullSnapshot {
            nodes: vec![],
            edges: vec![],
        };
        let json = serde_json::to_string(&evt).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["event"], "full_snapshot");
        assert_eq!(v["nodes"].as_array().unwrap().len(), 0);
        assert_eq!(v["edges"].as_array().unwrap().len(), 0);
    }
}
