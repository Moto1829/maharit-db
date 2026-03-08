pub mod algorithms;
pub mod concurrent_graph;
pub mod constraint;
pub mod fulltext;
pub mod graph_backend;
mod graph;
mod index;
mod property;
mod property_index;
pub mod traversal;

pub use algorithms::{
    DegreeCentrality, PageRankConfig, PageRankResult, betweenness_centrality, closeness_centrality,
    connected_components, find_cycles, has_cycle, label_propagation, pagerank,
    strongly_connected_components, topological_sort,
};
pub use constraint::{
    Constraint, ConstraintError, ConstraintManager, ConstraintType, PropertyType,
};
pub use fulltext::{FulltextError, FulltextIndex, FulltextManager, SearchResult};
pub use concurrent_graph::ConcurrentGraph;
pub use graph::{Edge, EdgeId, Graph, Node, NodeId};
pub use graph_backend::GraphBackend;
pub use index::LabelIndex;
pub use property::{temporal, PropertyValue};
pub use property_index::{IndexDefinition, PropertyIndex, PropertyKey};
pub use traversal::{
    AStar, Dijkstra, Direction, HeuristicFn, Path, Traversal, WeightFn, WeightedPath, all_paths,
    all_shortest_paths, has_path, shortest_path,
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
