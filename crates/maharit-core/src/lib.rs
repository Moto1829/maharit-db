pub mod algorithms;
pub mod constraint;
mod graph;
mod index;
mod property;
mod property_index;
pub mod traversal;

pub use algorithms::{
    DegreeCentrality, PageRankConfig, PageRankResult, betweenness_centrality, closeness_centrality,
    connected_components, find_cycles, has_cycle, pagerank, strongly_connected_components,
    topological_sort,
};
pub use constraint::{Constraint, ConstraintError, ConstraintManager, ConstraintType, PropertyType};
pub use graph::{Edge, EdgeId, Graph, Node, NodeId};
pub use index::LabelIndex;
pub use property::PropertyValue;
pub use property_index::{IndexDefinition, PropertyIndex, PropertyKey};
pub use traversal::{
    AStar, Dijkstra, Direction, HeuristicFn, Path, Traversal, WeightFn, WeightedPath, has_path,
    shortest_path,
};

use thiserror::Error;

/// グラフ操作のエラー型
#[derive(Debug, Clone, PartialEq, Error)]
pub enum GraphError {
    #[error("node not found: {0}")]
    NodeNotFound(NodeId),
    #[error("edge not found: {0}")]
    EdgeNotFound(EdgeId),
}
