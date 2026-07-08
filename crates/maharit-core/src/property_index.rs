//! Property-based index for fast lookups
//!
//! Provides indexing on node properties for:
//! - Exact match lookups
//! - Range queries on numeric properties

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::{NodeId, PropertyValue};

/// A key combining property name and value for exact match indexing
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PropertyKey {
    pub property: String,
    pub value: PropertyValue,
}

impl PropertyKey {
    pub fn new(property: impl Into<String>, value: PropertyValue) -> Self {
        Self {
            property: property.into(),
            value,
        }
    }
}

/// Index definition
#[derive(Debug, Clone)]
pub struct IndexDefinition {
    /// The label of nodes to index (e.g., "Person")
    pub label: String,
    /// The property name to index (e.g., "name")
    pub property: String,
    /// Whether this is a unique constraint
    pub unique: bool,
}

impl IndexDefinition {
    pub fn new(label: impl Into<String>, property: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            property: property.into(),
            unique: false,
        }
    }

    pub fn unique(label: impl Into<String>, property: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            property: property.into(),
            unique: true,
        }
    }

    /// Create index key for this definition
    pub fn key(&self) -> String {
        format!("{}:{}", self.label, self.property)
    }
}

/// Property index for exact match lookups
#[derive(Debug, Default, Clone)]
pub struct PropertyIndex {
    /// Index definitions: key -> definition
    definitions: HashMap<String, IndexDefinition>,
    /// Exact match index: (property_key) -> Set of NodeIds
    exact_index: HashMap<PropertyKey, HashSet<NodeId>>,
    /// Reverse index: NodeId -> Set of PropertyKeys (for cleanup)
    node_to_keys: HashMap<NodeId, HashSet<PropertyKey>>,
    /// Numeric range index: property_name -> BTreeMap<i64, Set<NodeId>>
    int_range_index: HashMap<String, BTreeMap<i64, HashSet<NodeId>>>,
    /// Float range index: property_name -> Vec<(f64, NodeId)> (sorted)
    float_range_index: HashMap<String, Vec<(OrderedFloat, NodeId)>>,
}

/// Wrapper for f64 to enable ordering
#[derive(Debug, Clone, Copy)]
struct OrderedFloat(f64);

impl PartialEq for OrderedFloat {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for OrderedFloat {}

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .partial_cmp(&other.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

impl PropertyIndex {
    pub fn new() -> Self {
        Self::default()
    }

    // ========== Index Management ==========

    /// Create a new index
    pub fn create_index(&mut self, definition: IndexDefinition) {
        let key = definition.key();
        self.definitions.insert(key, definition);
    }

    /// Drop an index
    pub fn drop_index(&mut self, label: &str, property: &str) -> bool {
        let key = format!("{}:{}", label, property);
        self.definitions.remove(&key).is_some()
    }

    /// Get all index definitions
    pub fn list_indexes(&self) -> Vec<&IndexDefinition> {
        self.definitions.values().collect()
    }

    /// Check if an index exists
    pub fn has_index(&self, label: &str, property: &str) -> bool {
        let key = format!("{}:{}", label, property);
        self.definitions.contains_key(&key)
    }

    // ========== Index Operations ==========

    /// Add a property value to the index
    pub fn index_property(&mut self, node_id: NodeId, property: &str, value: &PropertyValue) {
        let key = PropertyKey::new(property.to_string(), value.clone());

        // Add to exact index
        self.exact_index
            .entry(key.clone())
            .or_default()
            .insert(node_id);

        // Add to reverse index
        self.node_to_keys.entry(node_id).or_default().insert(key);

        // Add to range indexes if applicable
        match value {
            PropertyValue::Int(n) => {
                self.int_range_index
                    .entry(property.to_string())
                    .or_default()
                    .entry(*n)
                    .or_default()
                    .insert(node_id);
            }
            PropertyValue::Float(n) => {
                let entries = self
                    .float_range_index
                    .entry(property.to_string())
                    .or_default();
                entries.push((OrderedFloat(*n), node_id));
                entries.sort_by_key(|(f, _)| *f);
            }
            _ => {}
        }
    }

    /// Remove a property value from the index
    pub fn unindex_property(&mut self, node_id: NodeId, property: &str, value: &PropertyValue) {
        let key = PropertyKey::new(property.to_string(), value.clone());

        // Remove from exact index
        if let Some(nodes) = self.exact_index.get_mut(&key) {
            nodes.remove(&node_id);
            if nodes.is_empty() {
                self.exact_index.remove(&key);
            }
        }

        // Remove from reverse index
        if let Some(keys) = self.node_to_keys.get_mut(&node_id) {
            keys.remove(&key);
        }

        // Remove from range indexes
        match value {
            PropertyValue::Int(n) => {
                if let Some(btree) = self.int_range_index.get_mut(property)
                    && let Some(nodes) = btree.get_mut(n)
                {
                    nodes.remove(&node_id);
                    if nodes.is_empty() {
                        btree.remove(n);
                    }
                }
            }
            PropertyValue::Float(n) => {
                if let Some(entries) = self.float_range_index.get_mut(property) {
                    entries.retain(|(f, id)| !(f.0 == *n && *id == node_id));
                }
            }
            _ => {}
        }
    }

    /// Remove all indexed properties for a node
    pub fn remove_node(&mut self, node_id: NodeId) {
        if let Some(keys) = self.node_to_keys.remove(&node_id) {
            for key in keys {
                // Remove from exact index
                if let Some(nodes) = self.exact_index.get_mut(&key) {
                    nodes.remove(&node_id);
                    if nodes.is_empty() {
                        self.exact_index.remove(&key);
                    }
                }

                // Remove from range indexes
                match &key.value {
                    PropertyValue::Int(n) => {
                        if let Some(btree) = self.int_range_index.get_mut(&key.property)
                            && let Some(nodes) = btree.get_mut(n)
                        {
                            nodes.remove(&node_id);
                            if nodes.is_empty() {
                                btree.remove(n);
                            }
                        }
                    }
                    PropertyValue::Float(n) => {
                        if let Some(entries) = self.float_range_index.get_mut(&key.property) {
                            entries.retain(|(f, id)| !(f.0 == *n && *id == node_id));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // ========== Query Operations ==========

    /// Find nodes by exact property value match
    pub fn find_by_property(&self, property: &str, value: &PropertyValue) -> Vec<NodeId> {
        let key = PropertyKey::new(property.to_string(), value.clone());
        self.exact_index
            .get(&key)
            .map(|nodes| nodes.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Find nodes by integer property range (inclusive)
    pub fn find_by_int_range(&self, property: &str, min: i64, max: i64) -> Vec<NodeId> {
        let mut result = HashSet::new();

        if let Some(btree) = self.int_range_index.get(property) {
            for (_, nodes) in btree.range(min..=max) {
                result.extend(nodes.iter().copied());
            }
        }

        result.into_iter().collect()
    }

    /// Find nodes by float property range (inclusive)
    pub fn find_by_float_range(&self, property: &str, min: f64, max: f64) -> Vec<NodeId> {
        let mut result = Vec::new();

        if let Some(entries) = self.float_range_index.get(property) {
            for (value, node_id) in entries {
                if value.0 >= min && value.0 <= max {
                    result.push(*node_id);
                }
            }
        }

        result
    }

    /// Find nodes greater than a value
    pub fn find_greater_than(&self, property: &str, value: &PropertyValue) -> Vec<NodeId> {
        match value {
            PropertyValue::Int(n) => {
                let mut result = HashSet::new();
                if let Some(btree) = self.int_range_index.get(property) {
                    for (_, nodes) in btree.range((n + 1)..) {
                        result.extend(nodes.iter().copied());
                    }
                }
                result.into_iter().collect()
            }
            PropertyValue::Float(n) => {
                let mut result = Vec::new();
                if let Some(entries) = self.float_range_index.get(property) {
                    for (value, node_id) in entries {
                        if value.0 > *n {
                            result.push(*node_id);
                        }
                    }
                }
                result
            }
            _ => Vec::new(),
        }
    }

    /// Find nodes less than a value
    pub fn find_less_than(&self, property: &str, value: &PropertyValue) -> Vec<NodeId> {
        match value {
            PropertyValue::Int(n) => {
                let mut result = HashSet::new();
                if let Some(btree) = self.int_range_index.get(property) {
                    for (_, nodes) in btree.range(..*n) {
                        result.extend(nodes.iter().copied());
                    }
                }
                result.into_iter().collect()
            }
            PropertyValue::Float(n) => {
                let mut result = Vec::new();
                if let Some(entries) = self.float_range_index.get(property) {
                    for (value, node_id) in entries {
                        if value.0 < *n {
                            result.push(*node_id);
                        }
                    }
                }
                result
            }
            _ => Vec::new(),
        }
    }

    // ========== Statistics ==========

    /// Number of indexed property keys
    pub fn indexed_property_count(&self) -> usize {
        self.exact_index.len()
    }

    /// Number of nodes with indexed properties
    pub fn indexed_node_count(&self) -> usize {
        self.node_to_keys.len()
    }

    /// Clear all indexes
    pub fn clear(&mut self) {
        self.definitions.clear();
        self.exact_index.clear();
        self.node_to_keys.clear();
        self.int_range_index.clear();
        self.float_range_index.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_and_find() {
        let mut index = PropertyIndex::new();

        index.index_property(0, "name", &PropertyValue::String("Alice".to_string()));
        index.index_property(1, "name", &PropertyValue::String("Bob".to_string()));
        index.index_property(2, "name", &PropertyValue::String("Alice".to_string()));

        let alice_nodes =
            index.find_by_property("name", &PropertyValue::String("Alice".to_string()));
        assert_eq!(alice_nodes.len(), 2);
        assert!(alice_nodes.contains(&0));
        assert!(alice_nodes.contains(&2));

        let bob_nodes = index.find_by_property("name", &PropertyValue::String("Bob".to_string()));
        assert_eq!(bob_nodes.len(), 1);
        assert!(bob_nodes.contains(&1));
    }

    #[test]
    fn test_unindex_property() {
        let mut index = PropertyIndex::new();

        index.index_property(0, "name", &PropertyValue::String("Alice".to_string()));
        index.index_property(1, "name", &PropertyValue::String("Alice".to_string()));

        let nodes = index.find_by_property("name", &PropertyValue::String("Alice".to_string()));
        assert_eq!(nodes.len(), 2);

        index.unindex_property(0, "name", &PropertyValue::String("Alice".to_string()));

        let nodes = index.find_by_property("name", &PropertyValue::String("Alice".to_string()));
        assert_eq!(nodes.len(), 1);
        assert!(nodes.contains(&1));
    }

    #[test]
    fn test_remove_node() {
        let mut index = PropertyIndex::new();

        index.index_property(0, "name", &PropertyValue::String("Alice".to_string()));
        index.index_property(0, "age", &PropertyValue::Int(30));

        assert_eq!(index.indexed_node_count(), 1);

        index.remove_node(0);

        assert_eq!(index.indexed_node_count(), 0);
        assert!(
            index
                .find_by_property("name", &PropertyValue::String("Alice".to_string()))
                .is_empty()
        );
        assert!(
            index
                .find_by_property("age", &PropertyValue::Int(30))
                .is_empty()
        );
    }

    #[test]
    fn test_int_range_query() {
        let mut index = PropertyIndex::new();

        index.index_property(0, "age", &PropertyValue::Int(25));
        index.index_property(1, "age", &PropertyValue::Int(30));
        index.index_property(2, "age", &PropertyValue::Int(35));
        index.index_property(3, "age", &PropertyValue::Int(40));

        let range_28_35 = index.find_by_int_range("age", 28, 35);
        assert_eq!(range_28_35.len(), 2);
        assert!(range_28_35.contains(&1));
        assert!(range_28_35.contains(&2));

        let range_25_40 = index.find_by_int_range("age", 25, 40);
        assert_eq!(range_25_40.len(), 4);
    }

    #[test]
    fn test_float_range_query() {
        let mut index = PropertyIndex::new();

        index.index_property(0, "score", &PropertyValue::Float(1.5));
        index.index_property(1, "score", &PropertyValue::Float(2.5));
        index.index_property(2, "score", &PropertyValue::Float(3.5));

        let range = index.find_by_float_range("score", 2.0, 3.0);
        assert_eq!(range.len(), 1);
        assert!(range.contains(&1));

        let range = index.find_by_float_range("score", 1.0, 4.0);
        assert_eq!(range.len(), 3);
    }

    #[test]
    fn test_greater_than() {
        let mut index = PropertyIndex::new();

        index.index_property(0, "age", &PropertyValue::Int(25));
        index.index_property(1, "age", &PropertyValue::Int(30));
        index.index_property(2, "age", &PropertyValue::Int(35));

        let result = index.find_greater_than("age", &PropertyValue::Int(28));
        assert_eq!(result.len(), 2);
        assert!(result.contains(&1));
        assert!(result.contains(&2));
    }

    #[test]
    fn test_less_than() {
        let mut index = PropertyIndex::new();

        index.index_property(0, "age", &PropertyValue::Int(25));
        index.index_property(1, "age", &PropertyValue::Int(30));
        index.index_property(2, "age", &PropertyValue::Int(35));

        let result = index.find_less_than("age", &PropertyValue::Int(30));
        assert_eq!(result.len(), 1);
        assert!(result.contains(&0));
    }

    #[test]
    fn test_index_definition() {
        let mut index = PropertyIndex::new();

        let def = IndexDefinition::new("Person", "name");
        index.create_index(def);

        assert!(index.has_index("Person", "name"));
        assert!(!index.has_index("Person", "age"));

        let defs = index.list_indexes();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].label, "Person");
        assert_eq!(defs[0].property, "name");

        index.drop_index("Person", "name");
        assert!(!index.has_index("Person", "name"));
    }

    #[test]
    fn test_unique_index_definition() {
        let mut index = PropertyIndex::new();

        let def = IndexDefinition::unique("User", "email");
        index.create_index(def);

        let defs = index.list_indexes();
        assert_eq!(defs.len(), 1);
        assert!(defs[0].unique);
    }
}
