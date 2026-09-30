//! Mutation capture for WAL replication.
//!
//! [`RecordingGraph`] wraps a [`ConcurrentGraph`] as a [`GraphBackend`] and
//! records every mutation the executor performs as a [`WalEntryData`], in
//! execution order. The leader replicates exactly that log instead of diffing
//! the whole graph before/after each write, so updates to existing elements
//! (SET / REMOVE / label changes) reach followers and the cost is proportional
//! to the size of the write, not the size of the graph.

use maharit_core::{
    ConcurrentGraph, Edge, EdgeId, GraphBackend, GraphError, Node, NodeId, PropertyValue,
};

use crate::replication::WalEntryData;

/// Encode a property value for `WalEntryData::SetProperty` (JSON text).
///
/// Floats always carry a fractional part or exponent so the follower decodes
/// them as `Float` rather than `Int` (`1.0` must not become `1`).
pub fn property_value_to_wal_string(val: &PropertyValue) -> String {
    match val {
        PropertyValue::Null => "null".to_string(),
        PropertyValue::Bool(b) => b.to_string(),
        PropertyValue::Int(n) => n.to_string(),
        PropertyValue::Float(n) if n.is_finite() => format!("{n:?}"),
        PropertyValue::String(s) => serde_json::to_string(s).unwrap_or_default(),
        other => serde_json::to_string(&other.to_string()).unwrap_or_default(),
    }
}

/// A [`GraphBackend`] over a [`ConcurrentGraph`] that logs mutations.
pub struct RecordingGraph<'g> {
    graph: &'g ConcurrentGraph,
    /// When false, mutations are applied without being recorded (no replication).
    enabled: bool,
    log: Vec<WalEntryData>,
}

impl<'g> RecordingGraph<'g> {
    pub fn new(graph: &'g ConcurrentGraph, enabled: bool) -> Self {
        Self {
            graph,
            enabled,
            log: Vec::new(),
        }
    }

    /// Consume the wrapper and return the recorded mutations in order.
    pub fn into_log(self) -> Vec<WalEntryData> {
        self.log
    }

    fn record(&mut self, entry: impl FnOnce() -> WalEntryData) {
        if self.enabled {
            self.log.push(entry());
        }
    }
}

impl GraphBackend for RecordingGraph<'_> {
    fn get_node(&self, id: NodeId) -> Option<Node> {
        self.graph.get_node(id)
    }

    fn node_has_all_labels(&self, id: NodeId, labels: &[String]) -> bool {
        self.graph.node_has_all_labels(id, labels)
    }

    fn get_node_property(&self, id: NodeId, key: &str) -> Option<PropertyValue> {
        self.graph.get_node_property(id, key)
    }

    fn nodes_by_label(&self, label: &str) -> Vec<NodeId> {
        GraphBackend::nodes_by_label(self.graph, label)
    }

    fn get_edge(&self, id: EdgeId) -> Option<Edge> {
        self.graph.get_edge(id)
    }

    fn node_ids(&self) -> Vec<NodeId> {
        self.graph.node_ids()
    }

    fn all_nodes(&self) -> Vec<Node> {
        self.graph.all_nodes()
    }

    fn edge_ids(&self) -> Vec<EdgeId> {
        self.graph.edge_ids()
    }

    fn all_edges(&self) -> Vec<Edge> {
        self.graph.all_edges()
    }

    fn outgoing_edges(&self, node_id: NodeId) -> Vec<Edge> {
        self.graph.outgoing_edges(node_id)
    }

    fn incoming_edges(&self, node_id: NodeId) -> Vec<Edge> {
        self.graph.incoming_edges(node_id)
    }

    fn has_incident_edges(&self, node_id: NodeId) -> bool {
        self.graph.has_incident_edges(node_id)
    }

    fn node_count(&self) -> usize {
        GraphBackend::node_count(self.graph)
    }

    fn edge_count(&self) -> usize {
        GraphBackend::edge_count(self.graph)
    }

    fn contains_node(&self, id: NodeId) -> bool {
        GraphBackend::contains_node(self.graph, id)
    }

    fn create_node_with_labels(&mut self, labels: Vec<String>) -> NodeId {
        let id = self.graph.create_node_with_labels(labels.clone());
        self.record(|| WalEntryData::CreateNode {
            node_id: id,
            labels,
        });
        id
    }

    fn create_edge(
        &mut self,
        from: NodeId,
        to: NodeId,
        label: String,
    ) -> Result<EdgeId, GraphError> {
        let id = self.graph.create_edge(from, to, label.clone())?;
        self.record(|| WalEntryData::CreateEdge {
            edge_id: id,
            from,
            to,
            label,
        });
        Ok(id)
    }

    fn delete_node(&mut self, id: NodeId) -> Option<Node> {
        let removed = self.graph.delete_node(id);
        if removed.is_some() {
            self.record(|| WalEntryData::DeleteNode { node_id: id });
        }
        removed
    }

    fn delete_edge(&mut self, id: EdgeId) -> Option<Edge> {
        let removed = self.graph.delete_edge(id);
        if removed.is_some() {
            self.record(|| WalEntryData::DeleteEdge { edge_id: id });
        }
        removed
    }

    fn set_node_property(&mut self, id: NodeId, key: &str, value: PropertyValue) {
        let encoded = self.enabled.then(|| property_value_to_wal_string(&value));
        self.graph.set_node_property(id, key, value);
        if let Some(value) = encoded {
            self.record(|| WalEntryData::SetProperty {
                target_id: id,
                is_node: true,
                key: key.to_string(),
                value,
            });
        }
    }

    fn remove_node_property(&mut self, id: NodeId, key: &str) -> Option<PropertyValue> {
        let removed = self.graph.remove_node_property(id, key);
        if removed.is_some() {
            self.record(|| WalEntryData::RemoveProperty {
                target_id: id,
                is_node: true,
                key: key.to_string(),
            });
        }
        removed
    }

    fn add_node_label(&mut self, id: NodeId, label: String) {
        self.graph.add_node_label(id, label.clone());
        self.record(|| WalEntryData::AddLabel { node_id: id, label });
    }

    fn remove_node_label(&mut self, id: NodeId, label: &str) {
        self.graph.remove_node_label(id, label);
        self.record(|| WalEntryData::RemoveLabel {
            node_id: id,
            label: label.to_string(),
        });
    }

    fn set_edge_property(&mut self, id: EdgeId, key: &str, value: PropertyValue) {
        let encoded = self.enabled.then(|| property_value_to_wal_string(&value));
        self.graph.set_edge_property(id, key, value);
        if let Some(value) = encoded {
            self.record(|| WalEntryData::SetProperty {
                target_id: id,
                is_node: false,
                key: key.to_string(),
                value,
            });
        }
    }

    fn remove_edge_property(&mut self, id: EdgeId, key: &str) -> Option<PropertyValue> {
        let removed = self.graph.remove_edge_property(id, key);
        if removed.is_some() {
            self.record(|| WalEntryData::RemoveProperty {
                target_id: id,
                is_node: false,
                key: key.to_string(),
            });
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_mutations_in_order() {
        let graph = ConcurrentGraph::new();
        let mut rec = RecordingGraph::new(&graph, true);
        let a = rec.create_node_with_labels(vec!["P".into()]);
        let b = rec.create_node_with_labels(vec!["P".into()]);
        rec.set_node_property(a, "x", PropertyValue::Int(1));
        let e = rec.create_edge(a, b, "R".into()).unwrap();
        rec.set_edge_property(e, "w", PropertyValue::Float(1.0));
        rec.remove_node_property(a, "x");
        rec.remove_node_property(a, "missing"); // no-op: not recorded
        rec.add_node_label(a, "Q".into());
        rec.remove_node_label(a, "P");
        rec.delete_edge(e);
        rec.delete_node(b);

        let kinds: Vec<&str> = rec
            .into_log()
            .iter()
            .map(|e| match e {
                WalEntryData::CreateNode { .. } => "CreateNode",
                WalEntryData::DeleteNode { .. } => "DeleteNode",
                WalEntryData::CreateEdge { .. } => "CreateEdge",
                WalEntryData::DeleteEdge { .. } => "DeleteEdge",
                WalEntryData::SetProperty { .. } => "SetProperty",
                WalEntryData::RemoveProperty { .. } => "RemoveProperty",
                WalEntryData::AddLabel { .. } => "AddLabel",
                WalEntryData::RemoveLabel { .. } => "RemoveLabel",
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "CreateNode",
                "CreateNode",
                "SetProperty",
                "CreateEdge",
                "SetProperty",
                "RemoveProperty",
                "AddLabel",
                "RemoveLabel",
                "DeleteEdge",
                "DeleteNode",
            ]
        );
        // Mutations are applied to the underlying graph.
        assert!(graph.get_node(a).unwrap().has_label("Q"));
        assert!(graph.get_node(b).is_none());
    }

    #[test]
    fn disabled_recorder_applies_without_logging() {
        let graph = ConcurrentGraph::new();
        let mut rec = RecordingGraph::new(&graph, false);
        let a = rec.create_node_with_labels(vec!["P".into()]);
        rec.set_node_property(a, "x", PropertyValue::Int(1));
        assert!(rec.into_log().is_empty());
        assert_eq!(graph.get_node_property(a, "x"), Some(PropertyValue::Int(1)));
    }

    #[test]
    fn float_encoding_keeps_float_type() {
        assert_eq!(
            property_value_to_wal_string(&PropertyValue::Float(1.0)),
            "1.0"
        );
        assert_eq!(
            property_value_to_wal_string(&PropertyValue::Float(2.5)),
            "2.5"
        );
        assert_eq!(property_value_to_wal_string(&PropertyValue::Int(1)), "1");
    }
}
