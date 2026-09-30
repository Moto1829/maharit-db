//! Mutation capture for WAL replication and transaction undo.
//!
//! [`RecordingGraph`] wraps a [`ConcurrentGraph`] as a [`GraphBackend`] and
//! records every mutation the executor performs, in execution order:
//!
//! * as [`WalEntryData`] for the leader to replicate (instead of diffing the
//!   whole graph before/after each write), and optionally
//! * as [`UndoRecord`]s so a transaction can be rolled back (instead of
//!   snapshotting the whole graph before each statement).
//!
//! Both costs are proportional to the size of the write, not the graph.

use maharit_core::{
    ConcurrentGraph, Edge, EdgeId, GraphBackend, GraphError, Node, NodeId, PropertyValue,
};
use maharit_storage::UndoRecord;

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
    /// When false, mutations are applied without WAL recording (no replication).
    wal_enabled: bool,
    wal: Vec<WalEntryData>,
    /// `Some` when undo records should be captured (write inside a transaction).
    undo: Option<Vec<UndoRecord>>,
}

impl<'g> RecordingGraph<'g> {
    pub fn new(graph: &'g ConcurrentGraph, wal_enabled: bool) -> Self {
        Self {
            graph,
            wal_enabled,
            wal: Vec::new(),
            undo: None,
        }
    }

    /// Also capture undo records for transaction rollback.
    pub fn with_undo(mut self) -> Self {
        self.undo = Some(Vec::new());
        self
    }

    /// Consume the wrapper and return the recorded WAL entries in order.
    pub fn into_log(self) -> Vec<WalEntryData> {
        self.wal
    }

    /// Consume the wrapper and return the WAL entries and undo records, both
    /// in execution order.
    pub fn into_parts(self) -> (Vec<WalEntryData>, Vec<UndoRecord>) {
        (self.wal, self.undo.unwrap_or_default())
    }

    fn wal(&mut self, entry: impl FnOnce() -> WalEntryData) {
        if self.wal_enabled {
            self.wal.push(entry());
        }
    }

    fn undo(&mut self, record: impl FnOnce() -> UndoRecord) {
        if let Some(undo) = self.undo.as_mut() {
            undo.push(record());
        }
    }

    fn capture_undo(&self) -> bool {
        self.undo.is_some()
    }

    fn set_property_wal(&mut self, id: u64, is_node: bool, key: &str, value: &PropertyValue) {
        if self.wal_enabled {
            self.wal.push(WalEntryData::SetProperty {
                target_id: id,
                is_node,
                key: key.to_string(),
                value: property_value_to_wal_string(value),
            });
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
        self.wal(|| WalEntryData::CreateNode {
            node_id: id,
            labels,
        });
        self.undo(|| UndoRecord::CreateNode { node_id: id });
        id
    }

    fn create_edge(
        &mut self,
        from: NodeId,
        to: NodeId,
        label: String,
    ) -> Result<EdgeId, GraphError> {
        let id = self.graph.create_edge(from, to, label.clone())?;
        self.wal(|| WalEntryData::CreateEdge {
            edge_id: id,
            from,
            to,
            label,
        });
        self.undo(|| UndoRecord::CreateEdge { edge_id: id });
        Ok(id)
    }

    fn delete_node(&mut self, id: NodeId) -> Option<Node> {
        // `ConcurrentGraph::delete_node` also removes incident edges (DETACH);
        // capture them first so a rollback can restore them after the node.
        let incident: Vec<Edge> = if self.capture_undo() {
            let mut edges = self.graph.outgoing_edges(id);
            edges.extend(
                self.graph
                    .incoming_edges(id)
                    .into_iter()
                    .filter(|e| e.from != id), // self-loops are already in `outgoing`
            );
            edges
        } else {
            Vec::new()
        };

        let removed = self.graph.delete_node(id)?;
        self.wal(|| WalEntryData::DeleteNode { node_id: id });
        if let Some(undo) = self.undo.as_mut() {
            for e in incident {
                undo.push(UndoRecord::DeleteEdge {
                    edge_id: e.id,
                    from: e.from,
                    to: e.to,
                    label: e.label,
                    properties: e.properties,
                });
            }
            undo.push(UndoRecord::DeleteNode {
                node_id: id,
                labels: removed.labels.clone(),
                properties: removed.properties.clone(),
            });
        }
        Some(removed)
    }

    fn delete_edge(&mut self, id: EdgeId) -> Option<Edge> {
        let removed = self.graph.delete_edge(id)?;
        self.wal(|| WalEntryData::DeleteEdge { edge_id: id });
        self.undo(|| UndoRecord::DeleteEdge {
            edge_id: id,
            from: removed.from,
            to: removed.to,
            label: removed.label.clone(),
            properties: removed.properties.clone(),
        });
        Some(removed)
    }

    fn set_node_property(&mut self, id: NodeId, key: &str, value: PropertyValue) {
        if self.capture_undo() {
            let old_value = self.graph.get_node_property(id, key);
            self.undo(|| UndoRecord::SetProperty {
                node_id: id,
                key: key.to_string(),
                old_value,
            });
        }
        self.set_property_wal(id, true, key, &value);
        self.graph.set_node_property(id, key, value);
    }

    fn remove_node_property(&mut self, id: NodeId, key: &str) -> Option<PropertyValue> {
        let removed = self.graph.remove_node_property(id, key)?;
        self.wal(|| WalEntryData::RemoveProperty {
            target_id: id,
            is_node: true,
            key: key.to_string(),
        });
        self.undo(|| UndoRecord::SetProperty {
            node_id: id,
            key: key.to_string(),
            old_value: Some(removed.clone()),
        });
        Some(removed)
    }

    fn add_node_label(&mut self, id: NodeId, label: String) {
        let newly_added = self
            .graph
            .with_node(id, |n| !n.has_label(&label))
            .unwrap_or(false);
        self.graph.add_node_label(id, label.clone());
        if newly_added {
            self.undo(|| UndoRecord::AddLabel {
                node_id: id,
                label: label.clone(),
            });
            self.wal(|| WalEntryData::AddLabel { node_id: id, label });
        }
    }

    fn remove_node_label(&mut self, id: NodeId, label: &str) {
        let had_label = self
            .graph
            .with_node(id, |n| n.has_label(label))
            .unwrap_or(false);
        self.graph.remove_node_label(id, label);
        if had_label {
            self.wal(|| WalEntryData::RemoveLabel {
                node_id: id,
                label: label.to_string(),
            });
            self.undo(|| UndoRecord::RemoveLabel {
                node_id: id,
                label: label.to_string(),
            });
        }
    }

    fn set_edge_property(&mut self, id: EdgeId, key: &str, value: PropertyValue) {
        if self.capture_undo() {
            let old_value = self
                .graph
                .with_edge(id, |e| e.properties.get(key).cloned())
                .flatten();
            self.undo(|| UndoRecord::SetEdgeProperty {
                edge_id: id,
                key: key.to_string(),
                old_value,
            });
        }
        self.set_property_wal(id, false, key, &value);
        self.graph.set_edge_property(id, key, value);
    }

    fn remove_edge_property(&mut self, id: EdgeId, key: &str) -> Option<PropertyValue> {
        let removed = self.graph.remove_edge_property(id, key)?;
        self.wal(|| WalEntryData::RemoveProperty {
            target_id: id,
            is_node: false,
            key: key.to_string(),
        });
        self.undo(|| UndoRecord::SetEdgeProperty {
            edge_id: id,
            key: key.to_string(),
            old_value: Some(removed.clone()),
        });
        Some(removed)
    }

    fn restore_node(&mut self, id: NodeId, labels: Vec<String>) {
        self.graph
            .create_node_with_id_and_labels(id, labels.clone());
        self.wal(|| WalEntryData::CreateNode {
            node_id: id,
            labels,
        });
        self.undo(|| UndoRecord::CreateNode { node_id: id });
    }

    fn restore_edge(
        &mut self,
        id: EdgeId,
        from: NodeId,
        to: NodeId,
        label: String,
    ) -> Result<(), GraphError> {
        self.graph
            .create_edge_with_id(id, from, to, label.clone())?;
        self.wal(|| WalEntryData::CreateEdge {
            edge_id: id,
            from,
            to,
            label,
        });
        self.undo(|| UndoRecord::CreateEdge { edge_id: id });
        Ok(())
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

    /// Undo records captured by the recorder, applied through
    /// `TransactionManager::rollback_backend`, restore the exact prior state:
    /// original IDs, properties, labels and DETACH-deleted edges.
    #[test]
    fn undo_capture_round_trips_through_rollback() {
        use maharit_storage::TransactionManager;

        let graph = ConcurrentGraph::new();
        let a = graph.create_node_with_labels(vec!["P".into()]);
        let b = graph.create_node_with_labels(vec!["P".into()]);
        graph.set_node_property(a, "name", PropertyValue::String("a".into()));
        let e = graph.create_edge(a, b, "R").unwrap();
        graph.set_edge_property(e, "w", PropertyValue::Int(1));

        let tm = TransactionManager::new();
        let tx = tm.begin();
        let mut rec = RecordingGraph::new(&graph, false).with_undo();
        rec.set_node_property(a, "name", PropertyValue::String("changed".into()));
        rec.set_node_property(a, "new", PropertyValue::Int(1));
        rec.add_node_label(a, "Q".into());
        rec.remove_node_label(a, "P");
        rec.set_edge_property(e, "w", PropertyValue::Int(2));
        rec.delete_node(b); // DETACH: also removes edge e
        let c = rec.create_node_with_labels(vec!["C".into()]);
        let (_, undo) = rec.into_parts();
        tm.record_undo(tx, undo).unwrap();

        let mut rb = RecordingGraph::new(&graph, true);
        let touched = tm.rollback_backend(tx, &mut rb).unwrap();
        assert!(touched.contains(&a) && touched.contains(&b));

        let na = graph.get_node(a).unwrap();
        assert_eq!(
            na.properties.get("name"),
            Some(&PropertyValue::String("a".into()))
        );
        assert_eq!(na.properties.get("new"), None);
        assert!(na.has_label("P") && !na.has_label("Q"));
        assert!(
            graph.get_node(b).is_some(),
            "deleted node restored under its ID"
        );
        let edge = graph
            .get_edge(e)
            .expect("DETACH-deleted edge restored under its ID");
        assert_eq!((edge.from, edge.to), (a, b));
        assert_eq!(edge.properties.get("w"), Some(&PropertyValue::Int(1)));
        assert!(graph.get_node(c).is_none());

        // The compensating changes are themselves replicated.
        let wal = rb.into_log();
        assert!(
            wal.iter()
                .any(|w| matches!(w, WalEntryData::CreateNode { node_id, .. } if *node_id == b))
        );
        assert!(
            wal.iter()
                .any(|w| matches!(w, WalEntryData::CreateEdge { edge_id, .. } if *edge_id == e))
        );
        assert!(
            wal.iter()
                .any(|w| matches!(w, WalEntryData::DeleteNode { node_id } if *node_id == c))
        );
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
