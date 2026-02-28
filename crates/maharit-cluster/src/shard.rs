//! Shard metadata and the shard map that tracks all shards in the cluster.

use std::collections::HashMap;

use thiserror::Error;

/// Unique identifier for a shard within the cluster.
pub type ShardId = u32;

/// Lifecycle status of a shard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShardStatus {
    /// The shard is fully operational and accepts reads and writes.
    Active,
    /// The shard is unavailable (e.g., host down or not yet joined).
    Inactive,
    /// The shard is currently migrating data as part of a rebalance.
    Rebalancing,
}

/// Metadata describing a single shard.
#[derive(Debug, Clone)]
pub struct ShardInfo {
    /// Unique shard identifier.
    pub id: ShardId,
    /// Network address of the shard in `"host:port"` format.
    pub address: String,
    /// Current lifecycle status.
    pub status: ShardStatus,
    /// Approximate number of nodes stored in this shard.
    pub node_count: u64,
}

/// Errors that can occur during shard map operations.
#[derive(Debug, Error, PartialEq)]
pub enum ShardError {
    /// Attempted to add a shard with an ID that is already registered.
    #[error("shard {0} already exists")]
    AlreadyExists(ShardId),
    /// Attempted to remove or look up a shard that does not exist.
    #[error("shard {0} not found")]
    NotFound(ShardId),
}

/// A planned single-node migration between two shards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardMove {
    /// Identifier of the node to be relocated.
    pub node_id: u64,
    /// Shard the node is currently stored in.
    pub from_shard: ShardId,
    /// Shard the node should be relocated to.
    pub to_shard: ShardId,
}

/// A set of [`ShardMove`] operations that bring the cluster closer to balance.
#[derive(Debug, Default)]
pub struct RebalancePlan {
    /// Ordered list of moves to execute.
    pub moves: Vec<ShardMove>,
}

impl RebalancePlan {
    /// Return `true` if the plan contains no moves.
    pub fn is_empty(&self) -> bool {
        self.moves.is_empty()
    }
}

/// The authoritative registry of all shards in the cluster.
///
/// `ShardMap` does **not** hold graph data; it only tracks shard metadata.
#[derive(Debug, Default)]
pub struct ShardMap {
    shards: HashMap<ShardId, ShardInfo>,
}

impl ShardMap {
    /// Create an empty shard map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new shard.
    ///
    /// Returns [`ShardError::AlreadyExists`] if a shard with the same ID is
    /// already registered.
    pub fn add_shard(&mut self, info: ShardInfo) -> Result<(), ShardError> {
        if self.shards.contains_key(&info.id) {
            return Err(ShardError::AlreadyExists(info.id));
        }
        self.shards.insert(info.id, info);
        Ok(())
    }

    /// Deregister a shard by ID.
    ///
    /// Returns [`ShardError::NotFound`] if the shard is not registered.
    pub fn remove_shard(&mut self, id: ShardId) -> Result<(), ShardError> {
        if self.shards.remove(&id).is_none() {
            return Err(ShardError::NotFound(id));
        }
        Ok(())
    }

    /// Look up a shard by ID.
    pub fn get_shard(&self, id: ShardId) -> Option<&ShardInfo> {
        self.shards.get(&id)
    }

    /// Return mutable access to a shard by ID.
    pub fn get_shard_mut(&mut self, id: ShardId) -> Option<&mut ShardInfo> {
        self.shards.get_mut(&id)
    }

    /// Return all shards currently in [`ShardStatus::Active`] state.
    pub fn active_shards(&self) -> Vec<&ShardInfo> {
        self.shards
            .values()
            .filter(|s| s.status == ShardStatus::Active)
            .collect()
    }

    /// Return the total number of registered shards (regardless of status).
    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }

    /// Generate a simple rebalance plan.
    ///
    /// The algorithm computes the ideal number of nodes per shard, then for
    /// each over-loaded shard it emits [`ShardMove`] entries moving excess
    /// nodes to under-loaded shards.  This is a *planning* step only; no
    /// actual data is moved.
    ///
    /// Only [`ShardStatus::Active`] shards participate in rebalancing.
    pub fn rebalance_plan(&self) -> RebalancePlan {
        let active: Vec<&ShardInfo> = self.active_shards();
        if active.len() < 2 {
            return RebalancePlan::default();
        }

        let total_nodes: u64 = active.iter().map(|s| s.node_count).sum();
        let shard_count = active.len() as u64;
        let ideal = total_nodes / shard_count;

        // Sort for deterministic output (over-loaded shards first).
        let mut over: Vec<(ShardId, u64)> = active
            .iter()
            .filter(|s| s.node_count > ideal)
            .map(|s| (s.id, s.node_count - ideal))
            .collect();
        over.sort_by_key(|&(id, _)| id);

        let mut under: Vec<(ShardId, u64)> = active
            .iter()
            .filter(|s| s.node_count < ideal)
            .map(|s| (s.id, ideal - s.node_count))
            .collect();
        under.sort_by_key(|&(id, _)| id);

        let mut moves = Vec::new();
        let mut synthetic_node_id: u64 = 0;
        let mut over_iter = over.into_iter().peekable();
        let mut under_iter = under.into_iter().peekable();

        // Pair over-loaded shard capacity with under-loaded shard capacity.
        let mut over_rem: Option<(ShardId, u64)> = over_iter.next();
        let mut under_rem: Option<(ShardId, u64)> = under_iter.next();

        loop {
            match (over_rem, under_rem) {
                (Some((from, from_cap)), Some((to, to_cap))) => {
                    let batch = from_cap.min(to_cap);
                    for _ in 0..batch {
                        moves.push(ShardMove {
                            node_id: synthetic_node_id,
                            from_shard: from,
                            to_shard: to,
                        });
                        synthetic_node_id += 1;
                    }
                    over_rem = if from_cap > batch {
                        Some((from, from_cap - batch))
                    } else {
                        over_iter.next()
                    };
                    under_rem = if to_cap > batch {
                        Some((to, to_cap - batch))
                    } else {
                        under_iter.next()
                    };
                }
                _ => break,
            }
        }

        RebalancePlan { moves }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn active_shard(id: ShardId, node_count: u64) -> ShardInfo {
        ShardInfo {
            id,
            address: format!("127.0.0.1:{}", 7000 + id),
            status: ShardStatus::Active,
            node_count,
        }
    }

    fn inactive_shard(id: ShardId) -> ShardInfo {
        ShardInfo {
            id,
            address: format!("127.0.0.1:{}", 8000 + id),
            status: ShardStatus::Inactive,
            node_count: 0,
        }
    }

    // 1. Add a shard and retrieve it by ID.
    #[test]
    fn test_add_and_get_shard() {
        let mut map = ShardMap::new();
        map.add_shard(active_shard(1, 100)).unwrap();
        let info = map.get_shard(1).unwrap();
        assert_eq!(info.id, 1);
        assert_eq!(info.node_count, 100);
        assert_eq!(info.status, ShardStatus::Active);
    }

    // 2. Adding a duplicate shard ID returns AlreadyExists.
    #[test]
    fn test_add_duplicate_shard_error() {
        let mut map = ShardMap::new();
        map.add_shard(active_shard(1, 0)).unwrap();
        let err = map.add_shard(active_shard(1, 0)).unwrap_err();
        assert_eq!(err, ShardError::AlreadyExists(1));
    }

    // 3. Remove a shard and confirm it is gone.
    #[test]
    fn test_remove_shard() {
        let mut map = ShardMap::new();
        map.add_shard(active_shard(2, 50)).unwrap();
        assert_eq!(map.shard_count(), 1);
        map.remove_shard(2).unwrap();
        assert_eq!(map.shard_count(), 0);
        assert!(map.get_shard(2).is_none());
    }

    // 4. Removing a non-existent shard returns NotFound.
    #[test]
    fn test_remove_nonexistent_shard_error() {
        let mut map = ShardMap::new();
        let err = map.remove_shard(99).unwrap_err();
        assert_eq!(err, ShardError::NotFound(99));
    }

    // 5. active_shards() only returns shards in Active status.
    #[test]
    fn test_active_shards_filter() {
        let mut map = ShardMap::new();
        map.add_shard(active_shard(1, 10)).unwrap();
        map.add_shard(inactive_shard(2)).unwrap();
        map.add_shard(ShardInfo {
            id: 3,
            address: "127.0.0.1:9000".to_string(),
            status: ShardStatus::Rebalancing,
            node_count: 5,
        })
        .unwrap();

        let active = map.active_shards();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, 1);
    }

    // 6. shard_count() includes all registered shards regardless of status.
    #[test]
    fn test_shard_count() {
        let mut map = ShardMap::new();
        assert_eq!(map.shard_count(), 0);
        map.add_shard(active_shard(1, 0)).unwrap();
        map.add_shard(inactive_shard(2)).unwrap();
        assert_eq!(map.shard_count(), 2);
    }

    // 7. Rebalance plan is empty when there is only one active shard.
    #[test]
    fn test_rebalance_plan_single_shard() {
        let mut map = ShardMap::new();
        map.add_shard(active_shard(1, 100)).unwrap();
        let plan = map.rebalance_plan();
        assert!(plan.is_empty());
    }

    // 8. Rebalance plan is empty when all shards are already balanced.
    #[test]
    fn test_rebalance_plan_already_balanced() {
        let mut map = ShardMap::new();
        map.add_shard(active_shard(1, 50)).unwrap();
        map.add_shard(active_shard(2, 50)).unwrap();
        let plan = map.rebalance_plan();
        assert!(plan.is_empty());
    }

    // 9. Rebalance plan generates moves when shards are unbalanced.
    #[test]
    fn test_rebalance_plan_generates_moves() {
        let mut map = ShardMap::new();
        map.add_shard(active_shard(1, 100)).unwrap();
        map.add_shard(active_shard(2, 0)).unwrap();
        let plan = map.rebalance_plan();
        assert!(!plan.is_empty());
        // All moves should come from shard 1 and go to shard 2.
        for m in &plan.moves {
            assert_eq!(m.from_shard, 1);
            assert_eq!(m.to_shard, 2);
        }
    }

    // 10. Inactive shards are excluded from rebalance planning.
    #[test]
    fn test_rebalance_plan_skips_inactive() {
        let mut map = ShardMap::new();
        map.add_shard(active_shard(1, 100)).unwrap();
        map.add_shard(inactive_shard(2)).unwrap();
        // Only one active shard, so no moves expected.
        let plan = map.rebalance_plan();
        assert!(plan.is_empty());
    }

    // 11. get_shard returns None for unknown ID.
    #[test]
    fn test_get_shard_unknown() {
        let map = ShardMap::new();
        assert!(map.get_shard(42).is_none());
    }

    // 12. ShardMove fields are accessible.
    #[test]
    fn test_shard_move_fields() {
        let m = ShardMove {
            node_id: 7,
            from_shard: 1,
            to_shard: 3,
        };
        assert_eq!(m.node_id, 7);
        assert_eq!(m.from_shard, 1);
        assert_eq!(m.to_shard, 3);
    }
}
