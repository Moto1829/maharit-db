mod graph;
mod index;
mod property;
pub mod traversal;

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
