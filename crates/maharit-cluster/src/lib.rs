//! `maharit-cluster` — sharding and distributed query coordination for
//! MaharitDB.
//!
//! # Overview
//!
//! This crate provides:
//!
//! - **Sharding strategies** ([`strategy`]): [`HashSharding`],
//!   [`RangeSharding`], and [`LabelSharding`] that implement the
//!   [`ShardingStrategy`] trait.
//! - **Shard management** ([`shard`]): [`ShardMap`], [`ShardInfo`],
//!   [`ShardStatus`], [`RebalancePlan`], and error types.
//! - **Query routing** ([`router`]): [`QueryRouter`] and [`RoutingDecision`]
//!   for directing queries to the correct shard(s).
//! - **Cluster coordination** ([`coordinator`]): [`ClusterCoordinator`],
//!   [`ClusterConfig`], and result-merging utilities.

pub mod coordinator;
pub mod router;
pub mod shard;
pub mod strategy;

// ── Convenience re-exports ────────────────────────────────────────────────────

pub use coordinator::{ClusterCoordinator, ClusterConfig, Row, RowValue, ShardConfig};
pub use router::{EdgeLocation, QueryRouter, RoutingDecision, classify_edge};
pub use shard::{
    RebalancePlan, ShardError, ShardId, ShardInfo, ShardMap, ShardMove, ShardStatus,
};
pub use strategy::{HashSharding, LabelSharding, RangeSharding, ShardingStrategy};
