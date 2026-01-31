pub mod algorithms;
mod graph;
mod index;
mod property;
pub mod traversal;

pub use algorithms::{
    betweenness_centrality, closeness_centrality, connected_components, find_cycles, has_cycle,
    pagerank, strongly_connected_components, topological_sort, DegreeCentrality, PageRankConfig,
    PageRankResult,
};
pub use graph::{Edge, EdgeId, Graph, Node, NodeId};
pub use index::LabelIndex;
pub use property::PropertyValue;
pub use traversal::{
    has_path, shortest_path, AStar, Dijkstra, Direction, HeuristicFn, Path, Traversal, WeightFn,
    WeightedPath,
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
