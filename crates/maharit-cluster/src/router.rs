//! Query routing: determines which shard(s) a query must visit.

use std::sync::{Arc, RwLock};

use crate::shard::{ShardId, ShardMap};
use crate::strategy::ShardingStrategy;

/// The outcome of a routing decision for a single query operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingDecision {
    /// The query targets exactly one shard.
    SingleShard(ShardId),
    /// The query targets a specific subset of shards.
    MultiShard(Vec<ShardId>),
    /// The query must be broadcast to every active shard.
    Broadcast,
}

/// Where an edge's data lives relative to the cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeLocation {
    /// Both endpoints reside on the same shard.
    Local(ShardId),
    /// The source and destination endpoints reside on different shards.
    Remote { src: ShardId, dst: ShardId },
}

/// Routes queries to the appropriate shard(s) based on a [`ShardingStrategy`]
/// and the current [`ShardMap`].
pub struct QueryRouter {
    shard_map: Arc<RwLock<ShardMap>>,
    strategy: Box<dyn ShardingStrategy>,
}

impl QueryRouter {
    /// Create a new `QueryRouter`.
    pub fn new(shard_map: Arc<RwLock<ShardMap>>, strategy: Box<dyn ShardingStrategy>) -> Self {
        Self {
            shard_map,
            strategy,
        }
    }

    /// Determine the shard for a specific node.
    ///
    /// Uses the strategy to compute the shard ID; the shard ID is returned even
    /// if the shard is currently inactive (callers are responsible for handling
    /// that case).
    pub fn route_node_query(&self, node_id: u64, label: &str) -> ShardId {
        self.strategy.shard_for_node(node_id, label)
    }

    /// Return the shards that might contain nodes with the given label.
    ///
    /// For `LabelSharding` this can narrow down to a single shard.  For
    /// `HashSharding` or `RangeSharding` the label is irrelevant, so all
    /// active shard IDs are returned.
    pub fn route_label_query(&self, label: &str) -> Vec<ShardId> {
        // Probe with a representative node ID (0); if the strategy is
        // label-based it will return the mapped shard consistently regardless
        // of node_id.  For non-label-aware strategies we fall back to all
        // active shards.
        let probe_id: u64 = 0;
        let shard_a = self.strategy.shard_for_node(probe_id, label);
        let shard_b = self.strategy.shard_for_node(1, label);

        if shard_a == shard_b {
            // Same shard for different node IDs with same label → label-based
            // routing is in effect and we can target just that shard.
            vec![shard_a]
        } else {
            // Different shards → routing depends on node_id, so we must query
            // all active shards.
            self.route_all()
        }
    }

    /// Return the IDs of every active shard (for broadcast queries).
    pub fn route_all(&self) -> Vec<ShardId> {
        let map = self.shard_map.read().expect("shard_map lock poisoned");
        let mut ids: Vec<ShardId> = map.active_shards().iter().map(|s| s.id).collect();
        ids.sort_unstable();
        ids
    }

    /// Produce a [`RoutingDecision`] for a node-level query.
    pub fn decide_for_node(&self, node_id: u64, label: &str) -> RoutingDecision {
        RoutingDecision::SingleShard(self.route_node_query(node_id, label))
    }

    /// Produce a [`RoutingDecision`] for a label scan.
    pub fn decide_for_label(&self, label: &str) -> RoutingDecision {
        let shards = self.route_label_query(label);
        match shards.len() {
            0 => RoutingDecision::Broadcast,
            1 => RoutingDecision::SingleShard(shards[0]),
            _ => RoutingDecision::MultiShard(shards),
        }
    }

    /// Return a reference to the underlying strategy name.
    pub fn strategy_name(&self) -> &str {
        self.strategy.name()
    }
}

/// Determine whether an edge is local (both endpoints on the same shard) or
/// remote (endpoints on different shards).
pub fn classify_edge(
    src_node_id: u64,
    dst_node_id: u64,
    router: &QueryRouter,
    src_label: &str,
    dst_label: &str,
) -> EdgeLocation {
    let src_shard = router.route_node_query(src_node_id, src_label);
    let dst_shard = router.route_node_query(dst_node_id, dst_label);

    if src_shard == dst_shard {
        EdgeLocation::Local(src_shard)
    } else {
        EdgeLocation::Remote {
            src: src_shard,
            dst: dst_shard,
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shard::{ShardInfo, ShardStatus};
    use crate::strategy::{HashSharding, LabelSharding};

    fn make_router_hash(shard_count: usize) -> QueryRouter {
        let mut map = ShardMap::new();
        for i in 0..shard_count as u32 {
            map.add_shard(ShardInfo {
                id: i,
                address: format!("127.0.0.1:{}", 7000 + i),
                status: ShardStatus::Active,
                node_count: 0,
            })
            .unwrap();
        }
        QueryRouter::new(Arc::new(RwLock::new(map)), Box::new(HashSharding::new(shard_count)))
    }

    fn make_router_label() -> QueryRouter {
        let mut map = ShardMap::new();
        for id in 0..3u32 {
            map.add_shard(ShardInfo {
                id,
                address: format!("127.0.0.1:{}", 7000 + id),
                status: ShardStatus::Active,
                node_count: 0,
            })
            .unwrap();
        }
        let mut strategy = LabelSharding::new(0);
        strategy.add_label("Person", 1);
        strategy.add_label("City", 2);
        QueryRouter::new(Arc::new(RwLock::new(map)), Box::new(strategy))
    }

    // 1. route_node_query delegates to the strategy correctly.
    #[test]
    fn test_route_node_query_hash() {
        let router = make_router_hash(4);
        assert_eq!(router.route_node_query(0, "X"), 0);
        assert_eq!(router.route_node_query(5, "X"), 1);
    }

    // 2. route_all returns sorted active shard IDs.
    #[test]
    fn test_route_all_returns_active_shards() {
        let router = make_router_hash(3);
        let all = router.route_all();
        assert_eq!(all, vec![0, 1, 2]);
    }

    // 3. route_label_query with LabelSharding returns a single shard.
    #[test]
    fn test_route_label_query_label_sharding_single() {
        let router = make_router_label();
        let shards = router.route_label_query("Person");
        assert_eq!(shards, vec![1]);
    }

    // 4. route_label_query with HashSharding returns all active shards.
    #[test]
    fn test_route_label_query_hash_returns_all() {
        let router = make_router_hash(3);
        let shards = router.route_label_query("Person");
        assert_eq!(shards, vec![0, 1, 2]);
    }

    // 5. decide_for_node returns SingleShard.
    #[test]
    fn test_decide_for_node() {
        let router = make_router_hash(4);
        let decision = router.decide_for_node(4, "X"); // 4 % 4 == 0
        assert_eq!(decision, RoutingDecision::SingleShard(0));
    }

    // 6. decide_for_label with LabelSharding returns SingleShard.
    #[test]
    fn test_decide_for_label_single() {
        let router = make_router_label();
        let decision = router.decide_for_label("City");
        assert_eq!(decision, RoutingDecision::SingleShard(2));
    }

    // 7. decide_for_label with HashSharding returns MultiShard.
    #[test]
    fn test_decide_for_label_multi() {
        let router = make_router_hash(3);
        let decision = router.decide_for_label("Person");
        assert!(matches!(decision, RoutingDecision::MultiShard(_)));
    }

    // 8. classify_edge returns Local when both endpoints hash to same shard.
    #[test]
    fn test_classify_edge_local() {
        let router = make_router_hash(4);
        // node_id 0 and node_id 4 both go to shard 0 (mod 4).
        let loc = classify_edge(0, 4, &router, "X", "X");
        assert_eq!(loc, EdgeLocation::Local(0));
    }

    // 9. classify_edge returns Remote when endpoints hash to different shards.
    #[test]
    fn test_classify_edge_remote() {
        let router = make_router_hash(4);
        // node_id 0 → shard 0, node_id 1 → shard 1.
        let loc = classify_edge(0, 1, &router, "X", "X");
        assert_eq!(loc, EdgeLocation::Remote { src: 0, dst: 1 });
    }

    // 10. strategy_name is accessible.
    #[test]
    fn test_strategy_name() {
        let router = make_router_hash(2);
        assert_eq!(router.strategy_name(), "hash");

        let router_label = make_router_label();
        assert_eq!(router_label.strategy_name(), "label");
    }

    // 11. route_all returns empty when no active shards exist.
    #[test]
    fn test_route_all_no_active_shards() {
        let mut map = ShardMap::new();
        map.add_shard(ShardInfo {
            id: 0,
            address: "127.0.0.1:7000".to_string(),
            status: ShardStatus::Inactive,
            node_count: 0,
        })
        .unwrap();
        let router = QueryRouter::new(
            Arc::new(RwLock::new(map)),
            Box::new(HashSharding::new(1)),
        );
        assert!(router.route_all().is_empty());
    }
}
