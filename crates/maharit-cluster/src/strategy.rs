//! Sharding strategies for distributing nodes across shards.
//!
//! Three strategies are provided:
//! - [`HashSharding`]: Distributes nodes by `node_id % shard_count`.
//! - [`RangeSharding`]: Distributes nodes by matching their ID against
//!   registered `(min_id, max_id)` ranges.
//! - [`LabelSharding`]: Distributes nodes by their label, with a configurable
//!   default shard for unrecognized labels.

use std::collections::HashMap;

use crate::shard::ShardId;

/// Determines which shard a given node belongs to.
pub trait ShardingStrategy: Send + Sync {
    /// Return the shard ID for a node given its ID and label.
    fn shard_for_node(&self, node_id: u64, label: &str) -> ShardId;

    /// Human-readable name of this strategy.
    fn name(&self) -> &str;
}

// ─── Hash sharding ──────────────────────────────────────────────────────────

/// Assigns nodes to shards using `node_id % shard_count`.
///
/// This provides a roughly uniform distribution and requires no additional
/// configuration beyond the total number of shards.
#[derive(Debug, Clone)]
pub struct HashSharding {
    shard_count: usize,
}

impl HashSharding {
    /// Create a new `HashSharding` over `shard_count` shards.
    ///
    /// # Panics
    /// Panics if `shard_count` is zero.
    pub fn new(shard_count: usize) -> Self {
        assert!(shard_count > 0, "shard_count must be greater than zero");
        Self { shard_count }
    }

    /// Return the number of shards.
    pub fn shard_count(&self) -> usize {
        self.shard_count
    }
}

impl ShardingStrategy for HashSharding {
    fn shard_for_node(&self, node_id: u64, _label: &str) -> ShardId {
        (node_id % self.shard_count as u64) as ShardId
    }

    fn name(&self) -> &str {
        "hash"
    }
}

// ─── Range sharding ─────────────────────────────────────────────────────────

/// A single ID range entry: `[min_id, max_id]` maps to `shard_id`.
#[derive(Debug, Clone)]
pub struct RangeEntry {
    /// Inclusive lower bound.
    pub min_id: u64,
    /// Inclusive upper bound.
    pub max_id: u64,
    /// The shard that owns this range.
    pub shard_id: ShardId,
}

/// Assigns nodes to shards by matching their ID against ordered ranges.
///
/// Ranges are checked in insertion order; the first match wins.  If no range
/// matches, shard 0 is returned as the fallback.
#[derive(Debug, Clone)]
pub struct RangeSharding {
    ranges: Vec<RangeEntry>,
    default_shard: ShardId,
}

impl RangeSharding {
    /// Create a new `RangeSharding` with no ranges and the given default shard.
    pub fn new(default_shard: ShardId) -> Self {
        Self {
            ranges: Vec::new(),
            default_shard,
        }
    }

    /// Register a range `[min_id, max_id]` that maps to `shard_id`.
    pub fn add_range(&mut self, min_id: u64, max_id: u64, shard_id: ShardId) {
        self.ranges.push(RangeEntry {
            min_id,
            max_id,
            shard_id,
        });
    }

    /// Return a slice of all registered ranges.
    pub fn ranges(&self) -> &[RangeEntry] {
        &self.ranges
    }
}

impl ShardingStrategy for RangeSharding {
    fn shard_for_node(&self, node_id: u64, _label: &str) -> ShardId {
        for entry in &self.ranges {
            if node_id >= entry.min_id && node_id <= entry.max_id {
                return entry.shard_id;
            }
        }
        self.default_shard
    }

    fn name(&self) -> &str {
        "range"
    }
}

// ─── Label sharding ─────────────────────────────────────────────────────────

/// Assigns nodes to shards based on their label.
///
/// Labels not present in the map fall through to `default_shard`.
#[derive(Debug, Clone)]
pub struct LabelSharding {
    label_map: HashMap<String, ShardId>,
    default_shard: ShardId,
}

impl LabelSharding {
    /// Create a new `LabelSharding` with the given default shard.
    pub fn new(default_shard: ShardId) -> Self {
        Self {
            label_map: HashMap::new(),
            default_shard,
        }
    }

    /// Map `label` to `shard_id`.
    pub fn add_label(&mut self, label: impl Into<String>, shard_id: ShardId) {
        self.label_map.insert(label.into(), shard_id);
    }

    /// Return a reference to the label-to-shard map.
    pub fn label_map(&self) -> &HashMap<String, ShardId> {
        &self.label_map
    }

    /// Return the default shard ID used for unrecognised labels.
    pub fn default_shard(&self) -> ShardId {
        self.default_shard
    }
}

impl ShardingStrategy for LabelSharding {
    fn shard_for_node(&self, _node_id: u64, label: &str) -> ShardId {
        self.label_map
            .get(label)
            .copied()
            .unwrap_or(self.default_shard)
    }

    fn name(&self) -> &str {
        "label"
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── HashSharding ─────────────────────────────────────────────────────────

    #[test]
    fn test_hash_sharding_basic() {
        let s = HashSharding::new(4);
        assert_eq!(s.shard_for_node(0, "Person"), 0);
        assert_eq!(s.shard_for_node(1, "Person"), 1);
        assert_eq!(s.shard_for_node(2, "Person"), 2);
        assert_eq!(s.shard_for_node(3, "Person"), 3);
        assert_eq!(s.shard_for_node(4, "Person"), 0); // wraps
        assert_eq!(s.shard_for_node(7, "Person"), 3);
    }

    #[test]
    fn test_hash_sharding_single_shard() {
        let s = HashSharding::new(1);
        for id in 0..100 {
            assert_eq!(s.shard_for_node(id, "Any"), 0);
        }
    }

    #[test]
    fn test_hash_sharding_label_ignored() {
        let s = HashSharding::new(3);
        // Label should have no effect.
        assert_eq!(s.shard_for_node(6, "Person"), s.shard_for_node(6, "City"));
    }

    #[test]
    fn test_hash_sharding_name() {
        let s = HashSharding::new(4);
        assert_eq!(s.name(), "hash");
    }

    #[test]
    fn test_hash_sharding_shard_count() {
        let s = HashSharding::new(8);
        assert_eq!(s.shard_count(), 8);
    }

    #[test]
    #[should_panic(expected = "shard_count must be greater than zero")]
    fn test_hash_sharding_zero_count_panics() {
        let _ = HashSharding::new(0);
    }

    // ── RangeSharding ────────────────────────────────────────────────────────

    #[test]
    fn test_range_sharding_basic() {
        let mut s = RangeSharding::new(99);
        s.add_range(0, 99, 0);
        s.add_range(100, 199, 1);
        s.add_range(200, 299, 2);

        assert_eq!(s.shard_for_node(0, "X"), 0);
        assert_eq!(s.shard_for_node(50, "X"), 0);
        assert_eq!(s.shard_for_node(99, "X"), 0);
        assert_eq!(s.shard_for_node(100, "X"), 1);
        assert_eq!(s.shard_for_node(150, "X"), 1);
        assert_eq!(s.shard_for_node(199, "X"), 1);
        assert_eq!(s.shard_for_node(200, "X"), 2);
        assert_eq!(s.shard_for_node(299, "X"), 2);
    }

    #[test]
    fn test_range_sharding_default_shard() {
        let mut s = RangeSharding::new(5);
        s.add_range(0, 99, 0);
        // Node ID 500 falls outside all ranges.
        assert_eq!(s.shard_for_node(500, "X"), 5);
    }

    #[test]
    fn test_range_sharding_first_match_wins() {
        let mut s = RangeSharding::new(0);
        // Overlapping ranges: first registered wins.
        s.add_range(0, 100, 1);
        s.add_range(50, 100, 2);
        assert_eq!(s.shard_for_node(75, "X"), 1);
    }

    #[test]
    fn test_range_sharding_name() {
        let s = RangeSharding::new(0);
        assert_eq!(s.name(), "range");
    }

    #[test]
    fn test_range_sharding_boundary_inclusive() {
        let mut s = RangeSharding::new(9);
        s.add_range(10, 20, 3);
        assert_eq!(s.shard_for_node(10, "X"), 3);
        assert_eq!(s.shard_for_node(20, "X"), 3);
        assert_eq!(s.shard_for_node(9, "X"), 9);
        assert_eq!(s.shard_for_node(21, "X"), 9);
    }

    // ── LabelSharding ────────────────────────────────────────────────────────

    #[test]
    fn test_label_sharding_basic() {
        let mut s = LabelSharding::new(0);
        s.add_label("Person", 1);
        s.add_label("City", 2);
        s.add_label("Company", 3);

        assert_eq!(s.shard_for_node(42, "Person"), 1);
        assert_eq!(s.shard_for_node(42, "City"), 2);
        assert_eq!(s.shard_for_node(42, "Company"), 3);
    }

    #[test]
    fn test_label_sharding_default_for_unknown_label() {
        let mut s = LabelSharding::new(7);
        s.add_label("Known", 2);
        assert_eq!(s.shard_for_node(0, "Unknown"), 7);
    }

    #[test]
    fn test_label_sharding_node_id_ignored() {
        let mut s = LabelSharding::new(0);
        s.add_label("Tag", 5);
        // Different node IDs for same label must give same shard.
        assert_eq!(s.shard_for_node(1, "Tag"), 5);
        assert_eq!(s.shard_for_node(999, "Tag"), 5);
    }

    #[test]
    fn test_label_sharding_name() {
        let s = LabelSharding::new(0);
        assert_eq!(s.name(), "label");
    }

    #[test]
    fn test_label_sharding_overwrite() {
        let mut s = LabelSharding::new(0);
        s.add_label("Person", 1);
        s.add_label("Person", 4); // overwrite
        assert_eq!(s.shard_for_node(0, "Person"), 4);
    }
}
