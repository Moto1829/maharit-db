//! Schema definitions (constraints, property indexes, fulltext indexes) in a
//! form that can be saved in the data file, logged to the WAL and sent to
//! followers.
//!
//! The schema always travels whole: after any DDL statement the server logs
//! the complete new schema, and applying it replaces the previous one. That
//! keeps replay idempotent and needs no separate "drop" records.

use std::collections::BTreeSet;

use maharit_core::{
    ConcurrentGraph, Constraint, ConstraintManager, ConstraintType, FulltextManager, GraphBackend,
    IndexDefinition, NodeId, PropertyIndex, PropertyType,
};
use serde::{Deserialize, Serialize};

/// Every index and constraint definition of a database.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    #[serde(default)]
    pub constraints: Vec<ConstraintDef>,
    #[serde(default)]
    pub indexes: Vec<IndexDef>,
    #[serde(default)]
    pub fulltext: Vec<FulltextDef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstraintDef {
    pub name: String,
    pub label: String,
    pub properties: Vec<String>,
    pub kind: ConstraintKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConstraintKind {
    Unique,
    NotNull,
    /// `value_type` is one of `BOOLEAN`, `INTEGER`, `FLOAT`, `STRING`.
    Type {
        value_type: String,
    },
    RequiredLabel {
        label: String,
    },
    EndpointLabel {
        source_label: String,
        target_label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexDef {
    pub label: String,
    pub property: String,
    #[serde(default)]
    pub unique: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FulltextDef {
    pub name: String,
    pub label: String,
    pub properties: Vec<String>,
}

impl Schema {
    pub fn is_empty(&self) -> bool {
        self.constraints.is_empty() && self.indexes.is_empty() && self.fulltext.is_empty()
    }

    /// Read the definitions out of the managers (sorted, so equal schemas
    /// serialize identically).
    pub fn export(cm: &ConstraintManager, fm: &FulltextManager, pi: &PropertyIndex) -> Self {
        let mut constraints: Vec<ConstraintDef> = cm
            .list_constraints()
            .into_iter()
            .map(|c| ConstraintDef {
                name: c.name.clone(),
                label: c.label.clone(),
                properties: c.properties.clone(),
                kind: match &c.constraint_type {
                    ConstraintType::Unique => ConstraintKind::Unique,
                    ConstraintType::NotNull => ConstraintKind::NotNull,
                    ConstraintType::TypeCheck(t) => ConstraintKind::Type {
                        value_type: t.to_string(),
                    },
                    ConstraintType::RequiredLabel(l) => {
                        ConstraintKind::RequiredLabel { label: l.clone() }
                    }
                    ConstraintType::EndpointLabel {
                        source_label,
                        target_label,
                    } => ConstraintKind::EndpointLabel {
                        source_label: source_label.clone(),
                        target_label: target_label.clone(),
                    },
                },
            })
            .collect();
        constraints.sort_by(|a, b| a.name.cmp(&b.name));

        let mut indexes: Vec<IndexDef> = pi
            .list_indexes()
            .into_iter()
            .map(|d| IndexDef {
                label: d.label.clone(),
                property: d.property.clone(),
                unique: d.unique,
            })
            .collect();
        indexes.sort_by(|a, b| (&a.label, &a.property).cmp(&(&b.label, &b.property)));

        let mut fulltext: Vec<FulltextDef> = fm
            .list_indexes()
            .into_iter()
            .map(|i| FulltextDef {
                name: i.name().to_string(),
                label: i.label().to_string(),
                properties: i.properties().to_vec(),
            })
            .collect();
        fulltext.sort_by(|a, b| a.name.cmp(&b.name));

        Self {
            constraints,
            indexes,
            fulltext,
        }
    }

    /// Replace the managers' contents with this schema and build every index
    /// from the nodes in `graph`.
    pub fn install(
        &self,
        graph: &ConcurrentGraph,
        cm: &mut ConstraintManager,
        fm: &mut FulltextManager,
        pi: &mut PropertyIndex,
    ) -> Result<(), String> {
        let mut new_cm = ConstraintManager::new();
        for c in &self.constraints {
            new_cm
                .create_constraint(c.to_constraint()?)
                .map_err(|e| format!("constraint {}: {e}", c.name))?;
        }
        let mut new_pi = PropertyIndex::new();
        for d in &self.indexes {
            new_pi.create_index(if d.unique {
                IndexDefinition::unique(&d.label, &d.property)
            } else {
                IndexDefinition::new(&d.label, &d.property)
            });
        }
        let mut new_fm = FulltextManager::new();
        for f in &self.fulltext {
            new_fm
                .create_index(&f.name, &f.label, f.properties.clone())
                .map_err(|e| format!("fulltext index {}: {e}", f.name))?;
        }

        if !self.indexes.is_empty() || !self.fulltext.is_empty() {
            let fulltext_labels: BTreeSet<&str> =
                self.fulltext.iter().map(|f| f.label.as_str()).collect();
            for node in graph.nodes() {
                for d in &self.indexes {
                    if node.has_label(&d.label)
                        && let Some(v) = node.properties.get(&d.property)
                    {
                        new_pi.index_property(node.id, &d.property, v);
                    }
                }
                for label in &fulltext_labels {
                    if node.has_label(label) {
                        new_fm.index_node(node.id, label, &node.properties);
                    }
                }
            }
        }

        *cm = new_cm;
        *fm = new_fm;
        *pi = new_pi;
        Ok(())
    }
}

impl ConstraintDef {
    fn to_constraint(&self) -> Result<Constraint, String> {
        let constraint_type = match &self.kind {
            ConstraintKind::Unique => ConstraintType::Unique,
            ConstraintKind::NotNull => ConstraintType::NotNull,
            ConstraintKind::Type { value_type } => {
                ConstraintType::TypeCheck(match value_type.as_str() {
                    "BOOLEAN" => PropertyType::Bool,
                    "INTEGER" => PropertyType::Int,
                    "FLOAT" => PropertyType::Float,
                    "STRING" => PropertyType::String,
                    other => {
                        return Err(format!(
                            "constraint {}: unknown property type {other}",
                            self.name
                        ));
                    }
                })
            }
            ConstraintKind::RequiredLabel { label } => ConstraintType::RequiredLabel(label.clone()),
            ConstraintKind::EndpointLabel {
                source_label,
                target_label,
            } => ConstraintType::EndpointLabel {
                source_label: source_label.clone(),
                target_label: target_label.clone(),
            },
        };
        Ok(Constraint {
            name: self.name.clone(),
            label: self.label.clone(),
            properties: self.properties.clone(),
            constraint_type,
        })
    }
}

/// Replay WAL batches (from `DurableWal::open`) onto `graph`. Data changes
/// are applied as they come; the last logged schema replaces `schema`, and
/// its indexes are built once the server installs it.
pub fn replay_wal(
    graph: &ConcurrentGraph,
    schema: &mut Schema,
    batches: &[Vec<crate::replication::WalEntryData>],
) {
    use crate::replication::{WalEntryData, apply_wal_entry};
    for entry in batches.iter().flatten() {
        match entry {
            WalEntryData::SetSchema { schema: s } => *schema = s.clone(),
            other => apply_wal_entry(graph, other),
        }
    }
}

/// Bring the property and fulltext indexes of `ids` in line with the nodes'
/// current state in `graph` (after changes applied without the executor,
/// such as a rollback or replicated WAL entries).
pub fn reindex_nodes(
    graph: &ConcurrentGraph,
    ids: impl IntoIterator<Item = NodeId>,
    fm: &mut FulltextManager,
    pi: &mut PropertyIndex,
) {
    for id in ids {
        pi.remove_node(id);
        fm.remove_node(id);
        if let Some(node) = graph.get_node(id) {
            for label in &node.labels {
                for (key, value) in node.properties.iter() {
                    if pi.has_index(label, key) {
                        pi.index_property(id, key, value);
                    }
                }
                fm.index_node(id, label, &node.properties);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maharit_core::PropertyValue;

    fn sample() -> Schema {
        Schema {
            constraints: vec![
                ConstraintDef {
                    name: "age_int".into(),
                    label: "Person".into(),
                    properties: vec!["age".into()],
                    kind: ConstraintKind::Type {
                        value_type: "INTEGER".into(),
                    },
                },
                ConstraintDef {
                    name: "email_unique".into(),
                    label: "Person".into(),
                    properties: vec!["email".into()],
                    kind: ConstraintKind::Unique,
                },
                ConstraintDef {
                    name: "knows_ends".into(),
                    label: "KNOWS".into(),
                    properties: vec![],
                    kind: ConstraintKind::EndpointLabel {
                        source_label: "Person".into(),
                        target_label: "Person".into(),
                    },
                },
                ConstraintDef {
                    name: "need_label".into(),
                    label: "Employee".into(),
                    properties: vec![],
                    kind: ConstraintKind::RequiredLabel {
                        label: "Person".into(),
                    },
                },
                ConstraintDef {
                    name: "name_set".into(),
                    label: "Person".into(),
                    properties: vec!["name".into()],
                    kind: ConstraintKind::NotNull,
                },
            ],
            indexes: vec![IndexDef {
                label: "Person".into(),
                property: "name".into(),
                unique: false,
            }],
            fulltext: vec![FulltextDef {
                name: "bio".into(),
                label: "Person".into(),
                properties: vec!["bio".into()],
            }],
        }
    }

    #[test]
    fn install_then_export_round_trips_and_builds_indexes() {
        let graph = ConcurrentGraph::new();
        let a = graph.create_node("Person");
        graph.set_node_property(a, "name", PropertyValue::String("Alice".into()));
        graph.set_node_property(a, "bio", PropertyValue::String("graph databases".into()));
        let other = graph.create_node("City");
        graph.set_node_property(other, "name", PropertyValue::String("Alice".into()));

        let (mut cm, mut fm, mut pi) = (
            ConstraintManager::new(),
            FulltextManager::new(),
            PropertyIndex::new(),
        );
        let mut schema = sample();
        schema.install(&graph, &mut cm, &mut fm, &mut pi).unwrap();

        // Export sorts constraints by name.
        schema.constraints.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(Schema::export(&cm, &fm, &pi), schema);
        // Only the :Person node is in the index on :Person(name).
        assert_eq!(
            pi.find_by_property("name", &PropertyValue::String("Alice".into())),
            vec![a]
        );
        assert_eq!(fm.search("bio", "graph").unwrap().len(), 1);

        // JSON round trip.
        let json = serde_json::to_string(&schema).unwrap();
        assert_eq!(serde_json::from_str::<Schema>(&json).unwrap(), schema);

        // Installing an empty schema removes everything.
        Schema::default()
            .install(&graph, &mut cm, &mut fm, &mut pi)
            .unwrap();
        assert!(Schema::export(&cm, &fm, &pi).is_empty());
        assert!(!pi.has_any_index());
    }

    #[test]
    fn reindex_follows_the_current_node_state() {
        let graph = ConcurrentGraph::new();
        let a = graph.create_node("Person");
        graph.set_node_property(a, "name", PropertyValue::String("Old".into()));
        let (mut cm, mut fm, mut pi) = (
            ConstraintManager::new(),
            FulltextManager::new(),
            PropertyIndex::new(),
        );
        sample().install(&graph, &mut cm, &mut fm, &mut pi).unwrap();

        graph.set_node_property(a, "name", PropertyValue::String("New".into()));
        reindex_nodes(&graph, [a], &mut fm, &mut pi);
        assert!(
            pi.find_by_property("name", &PropertyValue::String("Old".into()))
                .is_empty()
        );
        assert_eq!(
            pi.find_by_property("name", &PropertyValue::String("New".into())),
            vec![a]
        );

        graph.delete_node(a);
        reindex_nodes(&graph, [a], &mut fm, &mut pi);
        assert!(
            pi.find_by_property("name", &PropertyValue::String("New".into()))
                .is_empty()
        );
    }
}
