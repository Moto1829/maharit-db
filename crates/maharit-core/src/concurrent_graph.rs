//! DashMap-based concurrent graph for lock-free concurrent reads and
//! fine-grained write locking.
//!
//! Unlike [`Graph`] (which uses a single `Vec<Option<Node>>`),
//! `ConcurrentGraph` stores nodes and edges in [`DashMap`] shards so that:
//!
//! - Multiple threads can **read** any entries concurrently without blocking
//!   each other.
//! - **Write** operations lock only the affected DashMap shard, not the entire
//!   graph, enabling higher write throughput on multi-core hardware.
//!
//! # Limitations vs [`Graph`]
//!
//! - No free-list reuse of deleted IDs (IDs are monotonically increasing).
//! - Adjacency sets (`outgoing_edges`, `incoming_edges`) still use `DashMap`
//!   shards, giving the same per-entry concurrency.
//!
//! # Usage
//!
//! ```
//! use maharit_core::ConcurrentGraph;
//!
//! let g = ConcurrentGraph::new();
//! let alice = g.create_node("Person");
//! let bob   = g.create_node("Person");
//! let edge  = g.create_edge(alice, bob, "KNOWS").unwrap();
//!
//! assert_eq!(g.node_count(), 2);
//! assert_eq!(g.edge_count(), 1);
//! ```

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;

use crate::graph::{Edge, EdgeId, Node, NodeId};
use crate::property::PropertyValue;
use crate::GraphError;

/// A lock-free, concurrent graph backed by [`DashMap`].
///
/// NodeIds and EdgeIds are assigned monotonically and never reused.
pub struct ConcurrentGraph {
    nodes: Arc<DashMap<NodeId, Node>>,
    edges: Arc<DashMap<EdgeId, Edge>>,
    /// Outgoing edge-id sets per node.
    outgoing: Arc<DashMap<NodeId, HashSet<EdgeId>>>,
    /// Incoming edge-id sets per node.
    incoming: Arc<DashMap<NodeId, HashSet<EdgeId>>>,
    next_node_id: AtomicU64,
    next_edge_id: AtomicU64,
}

impl Default for ConcurrentGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl ConcurrentGraph {
    /// Create an empty concurrent graph.
    pub fn new() -> Self {
        Self {
            nodes: Arc::new(DashMap::new()),
            edges: Arc::new(DashMap::new()),
            outgoing: Arc::new(DashMap::new()),
            incoming: Arc::new(DashMap::new()),
            next_node_id: AtomicU64::new(0),
            next_edge_id: AtomicU64::new(0),
        }
    }

    // ── Node operations ─────────────────────────────────────────────────────

    /// Create a new node with a single label and return its ID.
    pub fn create_node(&self, label: impl Into<String>) -> NodeId {
        let label_str = label.into();
        let labels = if label_str.is_empty() {
            vec![]
        } else {
            vec![label_str]
        };
        self.create_node_with_labels(labels)
    }

    /// Create a new node with multiple labels.
    pub fn create_node_with_labels(&self, labels: Vec<String>) -> NodeId {
        let id = self.next_node_id.fetch_add(1, Ordering::SeqCst);
        self.nodes.insert(
            id,
            Node {
                id,
                labels,
                properties: Arc::new(std::collections::HashMap::new()),
            },
        );
        self.outgoing.insert(id, HashSet::new());
        self.incoming.insert(id, HashSet::new());
        id
    }

    /// Return `true` if a node with this ID exists.
    pub fn contains_node(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    /// Read-only access to a node via a closure (holds a DashMap read guard).
    ///
    /// Returns `None` if the node does not exist.
    pub fn with_node<F, R>(&self, id: NodeId, f: F) -> Option<R>
    where
        F: FnOnce(&Node) -> R,
    {
        self.nodes.get(&id).map(|n| f(&n))
    }

    /// Mutable access to a node via a closure (holds a DashMap write guard).
    pub fn with_node_mut<F, R>(&self, id: NodeId, f: F) -> Option<R>
    where
        F: FnOnce(&mut Node) -> R,
    {
        self.nodes.get_mut(&id).map(|mut n| f(&mut n))
    }

    /// Remove a node and all its incident edges. Returns the removed node.
    pub fn delete_node(&self, id: NodeId) -> Option<Node> {
        let node = self.nodes.remove(&id).map(|(_, n)| n)?;

        // Remove all outgoing edges
        if let Some((_, out_ids)) = self.outgoing.remove(&id) {
            for eid in out_ids {
                if let Some((_, edge)) = self.edges.remove(&eid)
                    && let Some(mut inc) = self.incoming.get_mut(&edge.to) {
                        inc.remove(&eid);
                    }
            }
        }

        // Remove all incoming edges
        if let Some((_, in_ids)) = self.incoming.remove(&id) {
            for eid in in_ids {
                if let Some((_, edge)) = self.edges.remove(&eid)
                    && let Some(mut out) = self.outgoing.get_mut(&edge.from) {
                        out.remove(&eid);
                    }
            }
        }

        Some(node)
    }

    /// Number of live nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Iterate over all nodes (takes a short-lived read shard lock per entry).
    pub fn nodes(&self) -> impl Iterator<Item = dashmap::mapref::multiple::RefMulti<'_, NodeId, Node>> {
        self.nodes.iter()
    }

    // ── Edge operations ─────────────────────────────────────────────────────

    /// Create a directed edge between two existing nodes.
    pub fn create_edge(
        &self,
        from: NodeId,
        to: NodeId,
        label: impl Into<String>,
    ) -> Result<EdgeId, GraphError> {
        if !self.nodes.contains_key(&from) {
            return Err(GraphError::NodeNotFound(from));
        }
        if !self.nodes.contains_key(&to) {
            return Err(GraphError::NodeNotFound(to));
        }

        let id = self.next_edge_id.fetch_add(1, Ordering::SeqCst);
        self.edges.insert(
            id,
            Edge {
                id,
                label: label.into(),
                from,
                to,
                properties: Arc::new(std::collections::HashMap::new()),
            },
        );
        self.outgoing.entry(from).or_default().insert(id);
        self.incoming.entry(to).or_default().insert(id);
        Ok(id)
    }

    /// Read-only access to an edge via a closure.
    pub fn with_edge<F, R>(&self, id: EdgeId, f: F) -> Option<R>
    where
        F: FnOnce(&Edge) -> R,
    {
        self.edges.get(&id).map(|e| f(&e))
    }

    /// Mutable access to an edge via a closure (holds a DashMap write guard).
    pub fn with_edge_mut<F, R>(&self, id: EdgeId, f: F) -> Option<R>
    where
        F: FnOnce(&mut Edge) -> R,
    {
        self.edges.get_mut(&id).map(|mut e| f(&mut e))
    }

    /// Remove an edge by ID. Returns the removed edge.
    pub fn delete_edge(&self, id: EdgeId) -> Option<Edge> {
        let (_, edge) = self.edges.remove(&id)?;
        if let Some(mut out) = self.outgoing.get_mut(&edge.from) {
            out.remove(&id);
        }
        if let Some(mut inc) = self.incoming.get_mut(&edge.to) {
            inc.remove(&id);
        }
        Some(edge)
    }

    /// Number of live edges.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Iterate over all edges.
    pub fn edges(&self) -> impl Iterator<Item = dashmap::mapref::multiple::RefMulti<'_, EdgeId, Edge>> {
        self.edges.iter()
    }

    /// Set a property on a node.
    pub fn set_node_property(&self, id: NodeId, key: &str, value: PropertyValue) {
        if let Some(mut node) = self.nodes.get_mut(&id) {
            let mut props = (*node.properties).clone();
            props.insert(key.to_string(), value);
            node.properties = Arc::new(props);
        }
    }

    /// Set a property on an edge.
    pub fn set_edge_property(&self, id: EdgeId, key: &str, value: PropertyValue) {
        if let Some(mut edge) = self.edges.get_mut(&id) {
            let mut props = (*edge.properties).clone();
            props.insert(key.to_string(), value);
            edge.properties = Arc::new(props);
        }
    }

    /// Collect outgoing edges for a node into a Vec (snapshot).
    pub fn get_outgoing_edges(&self, node_id: NodeId) -> Vec<EdgeId> {
        self.outgoing
            .get(&node_id)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Collect incoming edges for a node into a Vec (snapshot).
    pub fn get_incoming_edges(&self, node_id: NodeId) -> Vec<EdgeId> {
        self.incoming
            .get(&node_id)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_query_nodes() {
        let g = ConcurrentGraph::new();
        let a = g.create_node("Person");
        let b = g.create_node("Person");
        assert_eq!(g.node_count(), 2);
        assert!(g.contains_node(a));
        assert!(g.contains_node(b));
        let name = g.with_node(a, |n| n.labels.clone()).unwrap();
        assert_eq!(name, vec!["Person"]);
    }

    #[test]
    fn test_create_edge() {
        let g = ConcurrentGraph::new();
        let a = g.create_node("A");
        let b = g.create_node("B");
        let e = g.create_edge(a, b, "KNOWS").unwrap();
        assert_eq!(g.edge_count(), 1);
        let label = g.with_edge(e, |edge| edge.label.clone()).unwrap();
        assert_eq!(label, "KNOWS");
    }

    #[test]
    fn test_edge_to_missing_node_fails() {
        let g = ConcurrentGraph::new();
        let a = g.create_node("A");
        assert!(matches!(
            g.create_edge(a, 999, "REL"),
            Err(GraphError::NodeNotFound(999))
        ));
    }

    #[test]
    fn test_delete_node_removes_incident_edges() {
        let g = ConcurrentGraph::new();
        let a = g.create_node("A");
        let b = g.create_node("B");
        g.create_edge(a, b, "KNOWS").unwrap();
        assert_eq!(g.edge_count(), 1);
        g.delete_node(a);
        assert_eq!(g.node_count(), 1);
        assert_eq!(g.edge_count(), 0);
    }

    #[test]
    fn test_delete_edge() {
        let g = ConcurrentGraph::new();
        let a = g.create_node("A");
        let b = g.create_node("B");
        let e = g.create_edge(a, b, "LINK").unwrap();
        g.delete_edge(e);
        assert_eq!(g.edge_count(), 0);
        assert!(g.get_outgoing_edges(a).is_empty());
    }

    #[test]
    fn test_set_node_property() {
        let g = ConcurrentGraph::new();
        let n = g.create_node("Person");
        g.set_node_property(n, "name", PropertyValue::String("Alice".to_string()));
        let name = g.with_node(n, |node| node.properties.get("name").cloned());
        assert_eq!(
            name.flatten(),
            Some(PropertyValue::String("Alice".to_string()))
        );
    }

    #[test]
    fn test_concurrent_reads() {
        use std::sync::Arc;
        use std::thread;

        let g = Arc::new(ConcurrentGraph::new());
        for _ in 0..10 {
            g.create_node("Node");
        }

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let g = Arc::clone(&g);
                thread::spawn(move || g.node_count())
            })
            .collect();

        for h in handles {
            assert_eq!(h.join().unwrap(), 10);
        }
    }

    #[test]
    fn test_adjacency_lists() {
        let g = ConcurrentGraph::new();
        let a = g.create_node("A");
        let b = g.create_node("B");
        let c = g.create_node("C");
        let e1 = g.create_edge(a, b, "R").unwrap();
        let e2 = g.create_edge(a, c, "R").unwrap();
        let out_a = g.get_outgoing_edges(a);
        assert!(out_a.contains(&e1));
        assert!(out_a.contains(&e2));
        let in_b = g.get_incoming_edges(b);
        assert!(in_b.contains(&e1));
    }

    #[test]
    fn test_multiple_labels() {
        let g = ConcurrentGraph::new();
        let n = g.create_node_with_labels(vec!["Person".to_string(), "Employee".to_string()]);
        let labels = g.with_node(n, |node| node.labels.clone()).unwrap();
        assert_eq!(labels.len(), 2);
        assert!(labels.contains(&"Person".to_string()));
        assert!(labels.contains(&"Employee".to_string()));
    }
}
