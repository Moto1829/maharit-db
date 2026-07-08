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
    /// Label → set of NodeIds carrying that label. Lets `MATCH (n:Label)`
    /// enumerate only matching nodes instead of scanning every node.
    /// Maintained on create/delete/label changes; rebuilt from creates on load.
    label_index: Arc<DashMap<String, HashSet<NodeId>>>,
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
            label_index: Arc::new(DashMap::new()),
            next_node_id: AtomicU64::new(0),
            next_edge_id: AtomicU64::new(0),
        }
    }

    // ── Label index maintenance ───────────────────────────────────────────────

    /// Register `id` under each of `labels` in the label index.
    fn label_index_add(&self, id: NodeId, labels: &[String]) {
        for label in labels {
            self.label_index.entry(label.clone()).or_default().insert(id);
        }
    }

    /// Remove `id` from each of `labels` in the label index.
    fn label_index_remove(&self, id: NodeId, labels: &[String]) {
        for label in labels {
            if let Some(mut set) = self.label_index.get_mut(label) {
                set.remove(&id);
            }
        }
    }

    /// Return the IDs of all nodes carrying `label` (index lookup, no scan).
    pub fn nodes_by_label(&self, label: &str) -> Vec<NodeId> {
        self.label_index
            .get(label)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Add a label to a node and update the label index.
    pub fn add_node_label(&self, id: NodeId, label: String) {
        let added = self
            .with_node_mut(id, |n| {
                if n.has_label(&label) {
                    false
                } else {
                    n.add_label(label.clone());
                    true
                }
            })
            .unwrap_or(false);
        if added {
            self.label_index.entry(label).or_default().insert(id);
        }
    }

    /// Remove a label from a node and update the label index.
    pub fn remove_node_label(&self, id: NodeId, label: &str) {
        let removed = self
            .with_node_mut(id, |n| {
                let had = n.has_label(label);
                if had {
                    n.remove_label(label);
                }
                had
            })
            .unwrap_or(false);
        if removed
            && let Some(mut set) = self.label_index.get_mut(label)
        {
            set.remove(&id);
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
        self.label_index_add(id, &labels);
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

    /// Create a node with a specific ID and multiple labels (for WAL replay).
    ///
    /// If the given `id` is greater than or equal to the current counter, the
    /// counter is advanced to `id + 1` so that subsequently created nodes do
    /// not collide with this one.
    pub fn create_node_with_id_and_labels(&self, id: NodeId, labels: Vec<String>) -> NodeId {
        // Advance next_node_id to at least id + 1 using a CAS loop.
        let mut current = self.next_node_id.load(Ordering::SeqCst);
        while current <= id {
            match self.next_node_id.compare_exchange(
                current,
                id + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(updated) => current = updated,
            }
        }
        self.label_index_add(id, &labels);
        let node = Node {
            id,
            labels,
            properties: Arc::new(std::collections::HashMap::new()),
        };
        self.nodes.insert(id, node);
        self.outgoing.entry(id).or_default();
        self.incoming.entry(id).or_default();
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

        // Remove from the label index.
        self.label_index_remove(id, &node.labels);

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

    fn sorted(mut v: Vec<NodeId>) -> Vec<NodeId> {
        v.sort_unstable();
        v
    }

    #[test]
    fn test_label_index_create_delete_and_labels() {
        let g = ConcurrentGraph::new();
        let a = g.create_node("Person");
        let b = g.create_node("Person");
        let c = g.create_node("City");

        assert_eq!(sorted(g.nodes_by_label("Person")), vec![a, b]);
        assert_eq!(g.nodes_by_label("City"), vec![c]);

        g.delete_node(a);
        assert_eq!(g.nodes_by_label("Person"), vec![b]);

        g.add_node_label(b, "Admin".to_string());
        assert_eq!(g.nodes_by_label("Admin"), vec![b]);
        g.add_node_label(b, "Admin".to_string()); // duplicate is a no-op
        assert_eq!(g.nodes_by_label("Admin"), vec![b]);

        g.remove_node_label(b, "Admin");
        assert!(g.nodes_by_label("Admin").is_empty());
        assert_eq!(g.nodes_by_label("Person"), vec![b]);
    }

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
