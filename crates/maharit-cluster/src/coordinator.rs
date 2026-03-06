//! Cluster coordinator: places nodes, executes local shard queries, and
//! merges results from multiple shards.
//!
//! Actual network communication is not performed here; shard execution is
//! simulated by operating on in-process [`Graph`] instances.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use maharit_core::{Graph, PropertyValue};

use crate::router::QueryRouter;
use crate::shard::{ShardId, ShardInfo, ShardMap, ShardStatus};
use crate::strategy::ShardingStrategy;

// ─── Configuration ───────────────────────────────────────────────────────────

/// Per-shard address configuration entry, deserializable from TOML.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShardConfig {
    /// Shard identifier.
    pub id: ShardId,
    /// Network address in `"host:port"` format.
    pub address: String,
}

/// Top-level cluster configuration, deserializable from TOML.
///
/// # Example (TOML)
/// ```toml
/// [sharding]
/// enabled = true
/// strategy = "hash"
/// replication_factor = 2
///
/// [[sharding.shards]]
/// id = 1
/// address = "node1:7687"
///
/// [[sharding.shards]]
/// id = 2
/// address = "node2:7687"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// Whether clustering / sharding is active.
    pub enabled: bool,
    /// Sharding strategy name: `"hash"`, `"range"`, or `"label"`.
    pub strategy: String,
    /// Ordered list of shard descriptors.
    pub shards: Vec<ShardConfig>,
    /// Number of replicas per shard (informational; not enforced here).
    pub replication_factor: u32,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            strategy: "hash".to_string(),
            shards: Vec::new(),
            replication_factor: 1,
        }
    }
}

// ─── Row type ────────────────────────────────────────────────────────────────

/// A single result row from a query, represented as a map of column name to
/// value.
///
/// A `Row` can represent a node ID, a label, or any scalar value returned by
/// a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Column values keyed by column name.
    pub columns: HashMap<String, RowValue>,
}

impl Row {
    /// Create an empty row.
    pub fn new() -> Self {
        Self {
            columns: HashMap::new(),
        }
    }

    /// Insert a value under the given column name.
    pub fn insert(&mut self, key: impl Into<String>, value: RowValue) {
        self.columns.insert(key.into(), value);
    }
}

impl Default for Row {
    fn default() -> Self {
        Self::new()
    }
}

/// A scalar value that can appear in a result row.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RowValue {
    /// Null / missing.
    Null,
    /// Integer value.
    Int(i64),
    /// String value.
    Text(String),
    /// Boolean value.
    Bool(bool),
}

impl From<&PropertyValue> for RowValue {
    fn from(pv: &PropertyValue) -> Self {
        match pv {
            PropertyValue::Null => RowValue::Null,
            PropertyValue::Int(v) => RowValue::Int(*v),
            PropertyValue::Float(v) => RowValue::Int(*v as i64),
            PropertyValue::String(s) => RowValue::Text(s.clone()),
            PropertyValue::Bool(b) => RowValue::Bool(*b),
            PropertyValue::Date(_) | PropertyValue::DateTime(_) | PropertyValue::Duration { .. } => {
                RowValue::Text(pv.to_string())
            }
        }
    }
}

// ─── ClusterCoordinator ──────────────────────────────────────────────────────

/// Coordinates distributed query execution across local in-process shards.
///
/// In a production implementation the coordinator would send sub-queries over
/// the network to remote shard nodes.  Here, each shard is represented by an
/// in-process [`Graph`] instance for testing purposes.
pub struct ClusterCoordinator {
    router: QueryRouter,
    /// Graphs managed locally, keyed by shard ID.
    local_shards: HashMap<ShardId, Arc<RwLock<Graph>>>,
}

impl ClusterCoordinator {
    /// Create a new coordinator.
    ///
    /// `config` is used to populate the shard map; the provided `strategy`
    /// drives routing decisions.  Each configured shard receives an empty
    /// local [`Graph`].
    pub fn new(config: ClusterConfig, strategy: Box<dyn ShardingStrategy>) -> Self {
        let mut shard_map = ShardMap::new();
        let mut local_shards: HashMap<ShardId, Arc<RwLock<Graph>>> = HashMap::new();

        for shard_cfg in &config.shards {
            let info = ShardInfo {
                id: shard_cfg.id,
                address: shard_cfg.address.clone(),
                status: ShardStatus::Active,
                node_count: 0,
            };
            // Ignore duplicate ID errors; first entry wins.
            let _ = shard_map.add_shard(info);
            local_shards
                .entry(shard_cfg.id)
                .or_insert_with(|| Arc::new(RwLock::new(Graph::new())));
        }

        let router = QueryRouter::new(Arc::new(RwLock::new(shard_map)), strategy);

        Self {
            router,
            local_shards,
        }
    }

    /// Determine which shard should store a node with the given ID and label.
    pub fn place_node(&self, node_id: u64, label: &str) -> ShardId {
        self.router.route_node_query(node_id, label)
    }

    /// Return the local [`Graph`] for `shard_id`, if this coordinator owns it.
    pub fn get_local_graph(&self, shard_id: ShardId) -> Option<Arc<RwLock<Graph>>> {
        self.local_shards.get(&shard_id).cloned()
    }

    /// Merge result rows from multiple shards.
    ///
    /// Rows are deduplicated by their entire column map so that cross-shard
    /// queries that would return the same row from multiple shards only produce
    /// one copy in the final result.
    pub fn merge_results(&self, results: Vec<Vec<Row>>) -> Vec<Row> {
        let mut seen: HashSet<Vec<(String, RowValue)>> = HashSet::new();
        let mut merged: Vec<Row> = Vec::new();

        for batch in results {
            for row in batch {
                // Produce a canonical sorted key for deduplication.
                let mut key: Vec<(String, RowValue)> = row.columns.clone().into_iter().collect();
                key.sort_by(|(a, _), (b, _)| a.cmp(b));
                if seen.insert(key) {
                    merged.push(row);
                }
            }
        }

        merged
    }

    /// Return metadata for all shards known to this coordinator.
    pub fn shard_info(&self) -> Vec<ShardInfo> {
        // Reconstruct shard infos from local shard graphs.
        self.local_shards
            .iter()
            .map(|(&id, graph)| {
                let node_count = graph
                    .read()
                    .map(|g| g.node_count() as u64)
                    .unwrap_or(0);
                ShardInfo {
                    id,
                    address: format!("local:{}", id),
                    status: ShardStatus::Active,
                    node_count,
                }
            })
            .collect()
    }

    /// Return `true` if both endpoints of an edge reside on the same shard.
    pub fn is_local_edge(&self, src_shard: ShardId, dst_shard: ShardId) -> bool {
        src_shard == dst_shard
    }

    /// Return a reference to the underlying [`QueryRouter`].
    pub fn router(&self) -> &QueryRouter {
        &self.router
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::HashSharding;

    fn make_coordinator(shard_count: usize) -> ClusterCoordinator {
        let shards: Vec<ShardConfig> = (0..shard_count as u32)
            .map(|id| ShardConfig {
                id,
                address: format!("127.0.0.1:{}", 7000 + id),
            })
            .collect();

        let config = ClusterConfig {
            enabled: true,
            strategy: "hash".to_string(),
            shards,
            replication_factor: 1,
        };
        let strategy = Box::new(HashSharding::new(shard_count));
        ClusterCoordinator::new(config, strategy)
    }

    // 1. place_node routes to the correct shard.
    #[test]
    fn test_place_node_hash() {
        let coord = make_coordinator(4);
        assert_eq!(coord.place_node(0, "Person"), 0);
        assert_eq!(coord.place_node(1, "Person"), 1);
        assert_eq!(coord.place_node(4, "Person"), 0); // 4 % 4
    }

    // 2. get_local_graph returns a graph for a valid shard.
    #[test]
    fn test_get_local_graph_exists() {
        let coord = make_coordinator(3);
        assert!(coord.get_local_graph(0).is_some());
        assert!(coord.get_local_graph(2).is_some());
    }

    // 3. get_local_graph returns None for an unknown shard.
    #[test]
    fn test_get_local_graph_unknown() {
        let coord = make_coordinator(2);
        assert!(coord.get_local_graph(99).is_none());
    }

    // 4. merge_results deduplicates identical rows.
    #[test]
    fn test_merge_results_deduplication() {
        let coord = make_coordinator(2);
        let mut row = Row::new();
        row.insert("id", RowValue::Int(1));

        let results = vec![vec![row.clone()], vec![row.clone()]];
        let merged = coord.merge_results(results);
        assert_eq!(merged.len(), 1);
    }

    // 5. merge_results keeps distinct rows from different shards.
    #[test]
    fn test_merge_results_distinct_rows() {
        let coord = make_coordinator(2);
        let mut row_a = Row::new();
        row_a.insert("id", RowValue::Int(1));
        let mut row_b = Row::new();
        row_b.insert("id", RowValue::Int(2));

        let results = vec![vec![row_a], vec![row_b]];
        let merged = coord.merge_results(results);
        assert_eq!(merged.len(), 2);
    }

    // 6. merge_results handles empty input.
    #[test]
    fn test_merge_results_empty() {
        let coord = make_coordinator(1);
        let merged = coord.merge_results(vec![]);
        assert!(merged.is_empty());
    }

    // 7. is_local_edge returns true for same shard.
    #[test]
    fn test_is_local_edge_true() {
        let coord = make_coordinator(3);
        assert!(coord.is_local_edge(1, 1));
    }

    // 8. is_local_edge returns false for different shards.
    #[test]
    fn test_is_local_edge_false() {
        let coord = make_coordinator(3);
        assert!(!coord.is_local_edge(0, 2));
    }

    // 9. shard_info returns one entry per local shard.
    #[test]
    fn test_shard_info_count() {
        let coord = make_coordinator(4);
        assert_eq!(coord.shard_info().len(), 4);
    }

    // 10. ClusterConfig deserializes from TOML.
    #[test]
    fn test_cluster_config_toml_deserialization() {
        let toml_str = r#"
            enabled = true
            strategy = "hash"
            replication_factor = 2

            [[shards]]
            id = 1
            address = "node1:7687"

            [[shards]]
            id = 2
            address = "node2:7687"

            [[shards]]
            id = 3
            address = "node3:7687"
        "#;

        let config: ClusterConfig = toml::from_str(toml_str).expect("TOML parse failed");
        assert!(config.enabled);
        assert_eq!(config.strategy, "hash");
        assert_eq!(config.replication_factor, 2);
        assert_eq!(config.shards.len(), 3);
        assert_eq!(config.shards[0].id, 1);
        assert_eq!(config.shards[0].address, "node1:7687");
        assert_eq!(config.shards[2].id, 3);
    }

    // 11. ClusterConfig can be round-tripped through TOML.
    #[test]
    fn test_cluster_config_toml_roundtrip() {
        let config = ClusterConfig {
            enabled: true,
            strategy: "label".to_string(),
            shards: vec![
                ShardConfig {
                    id: 0,
                    address: "localhost:7000".to_string(),
                },
                ShardConfig {
                    id: 1,
                    address: "localhost:7001".to_string(),
                },
            ],
            replication_factor: 3,
        };

        let serialized = toml::to_string(&config).unwrap();
        let deserialized: ClusterConfig = toml::from_str(&serialized).unwrap();

        assert_eq!(deserialized.enabled, config.enabled);
        assert_eq!(deserialized.strategy, config.strategy);
        assert_eq!(deserialized.replication_factor, config.replication_factor);
        assert_eq!(deserialized.shards.len(), config.shards.len());
        assert_eq!(deserialized.shards[0], config.shards[0]);
    }

    // 12. Row default is empty.
    #[test]
    fn test_row_default_empty() {
        let row = Row::default();
        assert!(row.columns.is_empty());
    }

    // 13. Coordinator constructed with zero shards has no local graphs.
    #[test]
    fn test_coordinator_zero_shards() {
        let config = ClusterConfig {
            enabled: false,
            strategy: "hash".to_string(),
            shards: vec![],
            replication_factor: 1,
        };
        let coord = ClusterCoordinator::new(config, Box::new(HashSharding::new(1)));
        assert_eq!(coord.shard_info().len(), 0);
    }
}
