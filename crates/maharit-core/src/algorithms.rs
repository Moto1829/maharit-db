//! Graph analysis algorithms
//!
//! This module provides various graph algorithms including:
//! - Centrality measures (degree, closeness, betweenness)
//! - PageRank
//! - Community detection (connected components, strongly connected components)
//! - Cycle detection and topological sort
//!
//! Algorithms operating on large graphs (>= 500 nodes) use rayon for parallel
//! computation where the algorithm structure allows it. For small graphs the
//! sequential path is taken to avoid rayon's thread-pool overhead.

use std::collections::{HashMap, HashSet, VecDeque};

use rayon::prelude::*;

use crate::{Graph, NodeId};

/// Threshold below which sequential algorithms are preferred over parallel ones.
/// Rayon's thread-pool overhead is not worth it for small graphs.
const PARALLEL_THRESHOLD: usize = 500;

// ============================================================================
// Centrality Measures
// ============================================================================

/// Degree centrality results
#[derive(Debug, Clone)]
pub struct DegreeCentrality {
    /// In-degree for each node
    pub in_degree: HashMap<NodeId, usize>,
    /// Out-degree for each node
    pub out_degree: HashMap<NodeId, usize>,
    /// Total degree (in + out) for each node
    pub total_degree: HashMap<NodeId, usize>,
}

impl DegreeCentrality {
    /// Calculate degree centrality for all nodes
    pub fn compute(graph: &Graph) -> Self {
        let mut in_degree = HashMap::new();
        let mut out_degree = HashMap::new();
        let mut total_degree = HashMap::new();

        // Initialize all nodes with 0
        for node in graph.nodes() {
            in_degree.insert(node.id, 0);
            out_degree.insert(node.id, 0);
            total_degree.insert(node.id, 0);
        }

        // Count degrees
        for edge in graph.edges() {
            *out_degree.get_mut(&edge.from).unwrap() += 1;
            *in_degree.get_mut(&edge.to).unwrap() += 1;
        }

        // Calculate total
        for node in graph.nodes() {
            let in_d = in_degree[&node.id];
            let out_d = out_degree[&node.id];
            total_degree.insert(node.id, in_d + out_d);
        }

        Self {
            in_degree,
            out_degree,
            total_degree,
        }
    }

    /// Get normalized degree centrality (0.0 to 1.0)
    pub fn normalized(&self, graph: &Graph) -> HashMap<NodeId, f64> {
        let n = graph.node_count();
        if n <= 1 {
            return self.total_degree.keys().map(|&id| (id, 0.0)).collect();
        }

        let max_degree = (n - 1) * 2; // Maximum possible degree in directed graph
        self.total_degree
            .iter()
            .map(|(&id, &deg)| (id, deg as f64 / max_degree as f64))
            .collect()
    }
}

/// Calculate closeness centrality for all nodes
///
/// Closeness centrality measures how close a node is to all other nodes.
/// Higher values indicate more central nodes.
///
/// For graphs with >= [`PARALLEL_THRESHOLD`] nodes the BFS computations for
/// each source node are executed in parallel using rayon.
pub fn closeness_centrality(graph: &Graph) -> HashMap<NodeId, f64> {
    let n = graph.node_count();

    if n <= 1 {
        return graph.nodes().map(|node| (node.id, 0.0)).collect();
    }

    let node_ids: Vec<NodeId> = graph.nodes().map(|n| n.id).collect();

    let compute_one = |&node_id: &NodeId| -> (NodeId, f64) {
        let distances = bfs_distances(graph, node_id);
        let reachable: usize = distances.values().filter(|&&d| d < usize::MAX).count();
        let total_distance: usize = distances.values().filter(|&&d| d < usize::MAX).sum();

        let c = if total_distance > 0 && reachable > 1 {
            (reachable - 1) as f64 / total_distance as f64
        } else {
            0.0
        };
        (node_id, c)
    };

    if n >= PARALLEL_THRESHOLD {
        node_ids.par_iter().map(compute_one).collect()
    } else {
        node_ids.iter().map(compute_one).collect()
    }
}

/// Calculate betweenness centrality for all nodes using Brandes' algorithm.
///
/// Betweenness centrality measures how often a node lies on shortest paths
/// between other nodes.
///
/// For graphs with >= [`PARALLEL_THRESHOLD`] nodes the outer loop over source
/// nodes is executed in parallel using rayon. Each thread accumulates a local
/// partial-centrality map which is then merged into the final result. Because
/// `Graph` is only accessed through a shared reference (`&Graph`) this is safe.
pub fn betweenness_centrality(graph: &Graph) -> HashMap<NodeId, f64> {
    let nodes: Vec<NodeId> = graph.nodes().map(|n| n.id).collect();
    let n = nodes.len();

    if n == 0 {
        return HashMap::new();
    }

    /// Compute partial betweenness contribution from a single source using Brandes.
    fn brandes_single(graph: &Graph, nodes: &[NodeId], source: NodeId) -> HashMap<NodeId, f64> {
        let mut stack = Vec::new();
        let mut predecessors: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let mut sigma: HashMap<NodeId, f64> = HashMap::new();
        let mut dist: HashMap<NodeId, i64> = HashMap::new();

        for &node in nodes {
            predecessors.insert(node, Vec::new());
            sigma.insert(node, 0.0);
            dist.insert(node, -1);
        }

        sigma.insert(source, 1.0);
        dist.insert(source, 0);

        let mut queue = VecDeque::new();
        queue.push_back(source);

        while let Some(v) = queue.pop_front() {
            stack.push(v);
            let v_dist = dist[&v];

            for edge in graph.get_outgoing_edges(v) {
                let w = edge.to;
                if dist[&w] < 0 {
                    dist.insert(w, v_dist + 1);
                    queue.push_back(w);
                }
                if dist[&w] == v_dist + 1 {
                    *sigma.get_mut(&w).unwrap() += sigma[&v];
                    predecessors.get_mut(&w).unwrap().push(v);
                }
            }
        }

        let mut delta: HashMap<NodeId, f64> = nodes.iter().map(|&nd| (nd, 0.0)).collect();
        let mut partial: HashMap<NodeId, f64> = nodes.iter().map(|&nd| (nd, 0.0)).collect();

        while let Some(w) = stack.pop() {
            for &v in &predecessors[&w] {
                let contribution = (sigma[&v] / sigma[&w]) * (1.0 + delta[&w]);
                *delta.get_mut(&v).unwrap() += contribution;
            }
            if w != source {
                *partial.get_mut(&w).unwrap() += delta[&w];
            }
        }

        partial
    }

    // Run the outer loop either in parallel or sequentially depending on graph size.
    let partial_maps: Vec<HashMap<NodeId, f64>> = if n >= PARALLEL_THRESHOLD {
        nodes
            .par_iter()
            .map(|&source| brandes_single(graph, &nodes, source))
            .collect()
    } else {
        nodes
            .iter()
            .map(|&source| brandes_single(graph, &nodes, source))
            .collect()
    };

    // Merge partial results into the final centrality map.
    let mut centrality: HashMap<NodeId, f64> = nodes.iter().map(|&nd| (nd, 0.0)).collect();
    for partial in partial_maps {
        for (node_id, value) in partial {
            *centrality.get_mut(&node_id).unwrap() += value;
        }
    }

    // Normalize for directed graph.
    let n_f = n as f64;
    if n_f > 2.0 {
        let norm = 1.0 / ((n_f - 1.0) * (n_f - 2.0));
        for value in centrality.values_mut() {
            *value *= norm;
        }
    }

    centrality
}

// ============================================================================
// PageRank
// ============================================================================

/// PageRank algorithm configuration
#[derive(Debug, Clone)]
pub struct PageRankConfig {
    /// Damping factor (typically 0.85)
    pub damping: f64,
    /// Maximum iterations
    pub max_iterations: usize,
    /// Convergence threshold
    pub tolerance: f64,
}

impl Default for PageRankConfig {
    fn default() -> Self {
        Self {
            damping: 0.85,
            max_iterations: 100,
            tolerance: 1e-6,
        }
    }
}

/// PageRank result
#[derive(Debug, Clone)]
pub struct PageRankResult {
    /// PageRank scores for each node
    pub scores: HashMap<NodeId, f64>,
    /// Number of iterations until convergence
    pub iterations: usize,
    /// Whether the algorithm converged
    pub converged: bool,
}

/// Calculate PageRank for all nodes.
///
/// For graphs with >= [`PARALLEL_THRESHOLD`] nodes the dangling-node sum is
/// computed in parallel. The per-iteration score update still involves a
/// sequential merge step (because edge contributions need atomic accumulation),
/// so only the embarrassingly-parallel parts are parallelised.
pub fn pagerank(graph: &Graph, config: &PageRankConfig) -> PageRankResult {
    let nodes: Vec<NodeId> = graph.nodes().map(|n| n.id).collect();
    let n = nodes.len();

    if n == 0 {
        return PageRankResult {
            scores: HashMap::new(),
            iterations: 0,
            converged: true,
        };
    }

    let use_parallel = n >= PARALLEL_THRESHOLD;

    let initial_score = 1.0 / n as f64;
    let mut scores: HashMap<NodeId, f64> = nodes.iter().map(|&id| (id, initial_score)).collect();

    // Build out-degree map (read-only after construction).
    let out_degree: HashMap<NodeId, usize> = nodes
        .iter()
        .map(|&id| (id, graph.get_outgoing_edges(id).len()))
        .collect();

    let damping = config.damping;
    let teleport = (1.0 - damping) / n as f64;

    // Pre-collect dangling node IDs (nodes with out-degree 0).
    let dangling_nodes: Vec<NodeId> = nodes
        .iter()
        .copied()
        .filter(|id| out_degree[id] == 0)
        .collect();

    let mut iterations = 0;
    let mut converged = false;

    for _ in 0..config.max_iterations {
        iterations += 1;
        let mut new_scores: HashMap<NodeId, f64> = nodes.iter().map(|&id| (id, teleport)).collect();

        // Calculate contribution from incoming edges (sequential — HashMap mutation).
        for edge in graph.edges() {
            let from_degree = out_degree[&edge.from];
            if from_degree > 0 {
                let contribution = damping * scores[&edge.from] / from_degree as f64;
                *new_scores.get_mut(&edge.to).unwrap() += contribution;
            }
        }

        // Handle dangling nodes (nodes with no outgoing edges).
        // The sum over dangling scores is embarrassingly parallel.
        let dangling_sum: f64 = if use_parallel {
            dangling_nodes
                .par_iter()
                .map(|id| scores[id])
                .sum()
        } else {
            dangling_nodes.iter().map(|id| scores[id]).sum()
        };
        let dangling_contribution = damping * dangling_sum / n as f64;

        for score in new_scores.values_mut() {
            *score += dangling_contribution;
        }

        // Check convergence (L1 norm of score difference).
        let diff: f64 = nodes
            .iter()
            .map(|&id| (new_scores[&id] - scores[&id]).abs())
            .sum();

        scores = new_scores;

        if diff < config.tolerance {
            converged = true;
            break;
        }
    }

    PageRankResult {
        scores,
        iterations,
        converged,
    }
}

// ============================================================================
// Community Detection
// ============================================================================

/// ラベル伝播法によるコミュニティ検出
///
/// 各ノードに最も多い隣接ノードのラベルを伝播させることで、
/// コミュニティ（クラスター）を検出する。
///
/// # Arguments
///
/// * `graph` - 対象グラフ
/// * `max_iterations` - 最大イテレーション数
///
/// # Returns
///
/// `HashMap<NodeId, CommunityId>` - 各ノードのコミュニティID
///
/// # Algorithm
///
/// 1. 初期化: 各ノードのコミュニティIDを自身のノードIDに設定
/// 2. 各イテレーション:
///    - 全ノードをノードID順に処理
///    - 各ノードの隣接ノード（出辺・入辺両方）のコミュニティIDをカウント
///    - 最多のコミュニティIDを自ノードのラベルとして採用（同数の場合は小さいIDを優先）
/// 3. ラベルが変化しなくなるか `max_iterations` に達したら終了
pub fn label_propagation(graph: &Graph, max_iterations: usize) -> HashMap<NodeId, u64> {
    let mut labels: HashMap<NodeId, u64> = graph.nodes().map(|n| (n.id, n.id)).collect();

    if labels.is_empty() {
        return labels;
    }

    let mut node_ids: Vec<NodeId> = graph.nodes().map(|n| n.id).collect();
    node_ids.sort_unstable();

    for _ in 0..max_iterations {
        let mut changed = false;

        for &node_id in &node_ids {
            // 隣接ノードのコミュニティIDをカウント（無向グラフとして扱う）
            let mut label_counts: HashMap<u64, usize> = HashMap::new();

            for edge in graph.get_outgoing_edges(node_id) {
                if let Some(&neighbor_label) = labels.get(&edge.to) {
                    *label_counts.entry(neighbor_label).or_insert(0) += 1;
                }
            }
            for edge in graph.get_incoming_edges(node_id) {
                if let Some(&neighbor_label) = labels.get(&edge.from) {
                    *label_counts.entry(neighbor_label).or_insert(0) += 1;
                }
            }

            if label_counts.is_empty() {
                // 孤立ノードは自分のコミュニティに留まる
                continue;
            }

            // 最多のラベルを選択。同数の場合は小さいIDを優先
            let max_count = *label_counts.values().max().unwrap();
            let best_label = label_counts
                .iter()
                .filter(|&(_, count)| *count == max_count)
                .map(|(&label, _)| label)
                .min()
                .unwrap();

            let current_label = labels[&node_id];
            if best_label != current_label {
                labels.insert(node_id, best_label);
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    labels
}

/// Find connected components in the graph (treating as undirected)
pub fn connected_components(graph: &Graph) -> Vec<HashSet<NodeId>> {
    let mut visited = HashSet::new();
    let mut components = Vec::new();

    for node in graph.nodes() {
        if !visited.contains(&node.id) {
            let mut component = HashSet::new();
            let mut queue = VecDeque::new();
            queue.push_back(node.id);

            while let Some(current) = queue.pop_front() {
                if visited.contains(&current) {
                    continue;
                }
                visited.insert(current);
                component.insert(current);

                // Add neighbors (both directions for undirected treatment)
                for edge in graph.get_outgoing_edges(current) {
                    if !visited.contains(&edge.to) {
                        queue.push_back(edge.to);
                    }
                }
                for edge in graph.get_incoming_edges(current) {
                    if !visited.contains(&edge.from) {
                        queue.push_back(edge.from);
                    }
                }
            }

            components.push(component);
        }
    }

    components
}

/// Find strongly connected components using Kosaraju's algorithm
pub fn strongly_connected_components(graph: &Graph) -> Vec<HashSet<NodeId>> {
    let nodes: Vec<NodeId> = graph.nodes().map(|n| n.id).collect();
    let mut visited = HashSet::new();
    let mut finish_order = Vec::new();

    // First DFS to get finish order
    for &node in &nodes {
        if !visited.contains(&node) {
            dfs_finish(graph, node, &mut visited, &mut finish_order);
        }
    }

    // Build reverse graph adjacency
    let mut reverse_adj: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for &node in &nodes {
        reverse_adj.insert(node, Vec::new());
    }
    for edge in graph.edges() {
        reverse_adj.get_mut(&edge.to).unwrap().push(edge.from);
    }

    // Second DFS on reverse graph in reverse finish order
    let mut visited = HashSet::new();
    let mut components = Vec::new();

    for &node in finish_order.iter().rev() {
        if !visited.contains(&node) {
            let mut component = HashSet::new();
            dfs_collect(&reverse_adj, node, &mut visited, &mut component);
            components.push(component);
        }
    }

    components
}

// ============================================================================
// Cycle Detection
// ============================================================================

/// Check if the graph contains a cycle
pub fn has_cycle(graph: &Graph) -> bool {
    let mut visited = HashSet::new();
    let mut rec_stack = HashSet::new();

    for node in graph.nodes() {
        if !visited.contains(&node.id)
            && dfs_cycle(graph, node.id, &mut visited, &mut rec_stack)
        {
            return true;
        }
    }

    false
}

/// Find all cycles in the graph (returns representative nodes from each cycle)
pub fn find_cycles(graph: &Graph) -> Vec<Vec<NodeId>> {
    // Use Johnson's algorithm for simple approach - find back edges
    let mut cycles = Vec::new();
    let mut visited = HashSet::new();
    let mut rec_stack = Vec::new();
    let mut in_stack = HashSet::new();

    for node in graph.nodes() {
        if !visited.contains(&node.id) {
            find_cycles_dfs(
                graph,
                node.id,
                &mut visited,
                &mut rec_stack,
                &mut in_stack,
                &mut cycles,
            );
        }
    }

    cycles
}

/// Perform topological sort (returns None if graph has a cycle)
pub fn topological_sort(graph: &Graph) -> Option<Vec<NodeId>> {
    let nodes: Vec<NodeId> = graph.nodes().map(|n| n.id).collect();

    // Calculate in-degrees
    let mut in_degree: HashMap<NodeId, usize> = nodes.iter().map(|&id| (id, 0)).collect();
    for edge in graph.edges() {
        *in_degree.get_mut(&edge.to).unwrap() += 1;
    }

    // Find nodes with no incoming edges
    let mut queue: VecDeque<NodeId> = nodes
        .iter()
        .filter(|&&id| in_degree[&id] == 0)
        .copied()
        .collect();

    let mut result = Vec::with_capacity(nodes.len());

    while let Some(node) = queue.pop_front() {
        result.push(node);

        for edge in graph.get_outgoing_edges(node) {
            let to_degree = in_degree.get_mut(&edge.to).unwrap();
            *to_degree -= 1;
            if *to_degree == 0 {
                queue.push_back(edge.to);
            }
        }
    }

    if result.len() == nodes.len() {
        Some(result)
    } else {
        None // Graph has a cycle
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

fn bfs_distances(graph: &Graph, source: NodeId) -> HashMap<NodeId, usize> {
    let mut distances: HashMap<NodeId, usize> = HashMap::new();

    for node in graph.nodes() {
        distances.insert(node.id, usize::MAX);
    }

    distances.insert(source, 0);
    let mut queue = VecDeque::new();
    queue.push_back(source);

    while let Some(current) = queue.pop_front() {
        let current_dist = distances[&current];

        for edge in graph.get_outgoing_edges(current) {
            if distances[&edge.to] == usize::MAX {
                distances.insert(edge.to, current_dist + 1);
                queue.push_back(edge.to);
            }
        }
    }

    distances
}

fn dfs_finish(
    graph: &Graph,
    node: NodeId,
    visited: &mut HashSet<NodeId>,
    finish_order: &mut Vec<NodeId>,
) {
    visited.insert(node);

    for edge in graph.get_outgoing_edges(node) {
        if !visited.contains(&edge.to) {
            dfs_finish(graph, edge.to, visited, finish_order);
        }
    }

    finish_order.push(node);
}

fn dfs_collect(
    adj: &HashMap<NodeId, Vec<NodeId>>,
    node: NodeId,
    visited: &mut HashSet<NodeId>,
    component: &mut HashSet<NodeId>,
) {
    visited.insert(node);
    component.insert(node);

    if let Some(neighbors) = adj.get(&node) {
        for &neighbor in neighbors {
            if !visited.contains(&neighbor) {
                dfs_collect(adj, neighbor, visited, component);
            }
        }
    }
}

fn dfs_cycle(
    graph: &Graph,
    node: NodeId,
    visited: &mut HashSet<NodeId>,
    rec_stack: &mut HashSet<NodeId>,
) -> bool {
    visited.insert(node);
    rec_stack.insert(node);

    for edge in graph.get_outgoing_edges(node) {
        if !visited.contains(&edge.to) {
            if dfs_cycle(graph, edge.to, visited, rec_stack) {
                return true;
            }
        } else if rec_stack.contains(&edge.to) {
            return true;
        }
    }

    rec_stack.remove(&node);
    false
}

fn find_cycles_dfs(
    graph: &Graph,
    node: NodeId,
    visited: &mut HashSet<NodeId>,
    rec_stack: &mut Vec<NodeId>,
    in_stack: &mut HashSet<NodeId>,
    cycles: &mut Vec<Vec<NodeId>>,
) {
    visited.insert(node);
    rec_stack.push(node);
    in_stack.insert(node);

    for edge in graph.get_outgoing_edges(node) {
        if !visited.contains(&edge.to) {
            find_cycles_dfs(graph, edge.to, visited, rec_stack, in_stack, cycles);
        } else if in_stack.contains(&edge.to) {
            // Found a cycle
            let cycle_start = rec_stack.iter().position(|&n| n == edge.to).unwrap();
            let cycle: Vec<NodeId> = rec_stack[cycle_start..].to_vec();
            cycles.push(cycle);
        }
    }

    rec_stack.pop();
    in_stack.remove(&node);
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_simple_graph() -> Graph {
        // A -> B -> C
        // |         |
        // v         v
        // D ------> E
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        let c = graph.create_node("C");
        let d = graph.create_node("D");
        let e = graph.create_node("E");

        graph.create_edge(a, b, "to").unwrap();
        graph.create_edge(b, c, "to").unwrap();
        graph.create_edge(a, d, "to").unwrap();
        graph.create_edge(c, e, "to").unwrap();
        graph.create_edge(d, e, "to").unwrap();

        graph
    }

    fn create_cyclic_graph() -> Graph {
        // A -> B -> C -> A (cycle)
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        let c = graph.create_node("C");

        graph.create_edge(a, b, "to").unwrap();
        graph.create_edge(b, c, "to").unwrap();
        graph.create_edge(c, a, "to").unwrap();

        graph
    }

    #[test]
    fn test_degree_centrality() {
        let graph = create_simple_graph();
        let centrality = DegreeCentrality::compute(&graph);

        // Node A has out-degree 2
        assert_eq!(centrality.out_degree[&0], 2);
        // Node E has in-degree 2
        assert_eq!(centrality.in_degree[&4], 2);
    }

    #[test]
    fn test_pagerank() {
        let graph = create_simple_graph();
        let result = pagerank(&graph, &PageRankConfig::default());

        assert!(result.converged);
        // All scores should sum to approximately 1.0
        let sum: f64 = result.scores.values().sum();
        assert!((sum - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_connected_components() {
        let mut graph = Graph::new();
        // Component 1: A -> B
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        graph.create_edge(a, b, "to").unwrap();

        // Component 2: C -> D
        let c = graph.create_node("C");
        let d = graph.create_node("D");
        graph.create_edge(c, d, "to").unwrap();

        let components = connected_components(&graph);
        assert_eq!(components.len(), 2);
    }

    #[test]
    fn test_strongly_connected_components() {
        let graph = create_cyclic_graph();
        let sccs = strongly_connected_components(&graph);

        // All nodes should be in one SCC due to cycle
        assert_eq!(sccs.len(), 1);
        assert_eq!(sccs[0].len(), 3);
    }

    #[test]
    fn test_has_cycle() {
        let acyclic = create_simple_graph();
        assert!(!has_cycle(&acyclic));

        let cyclic = create_cyclic_graph();
        assert!(has_cycle(&cyclic));
    }

    #[test]
    fn test_topological_sort() {
        let acyclic = create_simple_graph();
        let sorted = topological_sort(&acyclic);
        assert!(sorted.is_some());
        let sorted = sorted.unwrap();
        assert_eq!(sorted.len(), 5);

        // A should come before B, C, D, E
        let a_pos = sorted.iter().position(|&n| n == 0).unwrap();
        let b_pos = sorted.iter().position(|&n| n == 1).unwrap();
        assert!(a_pos < b_pos);

        let cyclic = create_cyclic_graph();
        assert!(topological_sort(&cyclic).is_none());
    }

    #[test]
    fn test_find_cycles() {
        let cyclic = create_cyclic_graph();
        let cycles = find_cycles(&cyclic);
        assert!(!cycles.is_empty());
    }

    #[test]
    fn test_closeness_centrality() {
        let graph = create_simple_graph();
        let centrality = closeness_centrality(&graph);

        // All nodes should have some centrality value
        assert_eq!(centrality.len(), 5);
        for &value in centrality.values() {
            assert!(value >= 0.0);
        }
    }

    #[test]
    fn test_betweenness_centrality() {
        let graph = create_simple_graph();
        let centrality = betweenness_centrality(&graph);

        // All nodes should have some centrality value
        assert_eq!(centrality.len(), 5);
        for &value in centrality.values() {
            assert!(value >= 0.0);
        }
    }

    // ============================================================================
    // Label Propagation Tests
    // ============================================================================

    #[test]
    fn test_label_propagation_isolated_node() {
        // 孤立ノードは自分のコミュニティに所属する
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        // エッジなし（孤立ノード）

        let communities = label_propagation(&graph, 10);

        assert_eq!(communities.len(), 2);
        // 各ノードは自分自身のコミュニティ（ID）に所属する
        assert_eq!(communities[&a], a);
        assert_eq!(communities[&b], b);
    }

    #[test]
    fn test_label_propagation_complete_graph() {
        // 完全グラフは1コミュニティになる
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        let c = graph.create_node("C");

        graph.create_edge(a, b, "KNOWS").unwrap();
        graph.create_edge(b, a, "KNOWS").unwrap();
        graph.create_edge(b, c, "KNOWS").unwrap();
        graph.create_edge(c, b, "KNOWS").unwrap();
        graph.create_edge(a, c, "KNOWS").unwrap();
        graph.create_edge(c, a, "KNOWS").unwrap();

        let communities = label_propagation(&graph, 50);

        // 全ノードが同じコミュニティに収束するはず
        let unique_communities: std::collections::HashSet<u64> =
            communities.values().copied().collect();
        assert_eq!(unique_communities.len(), 1, "完全グラフは1コミュニティになるべき");
    }

    #[test]
    fn test_label_propagation_two_clusters() {
        // 2つのクラスターが正しく分離される
        // クラスター1: A-B-C（互いに密接）
        // クラスター2: D-E-F（互いに密接）
        // A と D はエッジなし
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        let c = graph.create_node("C");
        let d = graph.create_node("D");
        let e = graph.create_node("E");
        let f = graph.create_node("F");

        // クラスター1内の接続
        graph.create_edge(a, b, "KNOWS").unwrap();
        graph.create_edge(b, a, "KNOWS").unwrap();
        graph.create_edge(b, c, "KNOWS").unwrap();
        graph.create_edge(c, b, "KNOWS").unwrap();
        graph.create_edge(a, c, "KNOWS").unwrap();
        graph.create_edge(c, a, "KNOWS").unwrap();

        // クラスター2内の接続
        graph.create_edge(d, e, "KNOWS").unwrap();
        graph.create_edge(e, d, "KNOWS").unwrap();
        graph.create_edge(e, f, "KNOWS").unwrap();
        graph.create_edge(f, e, "KNOWS").unwrap();
        graph.create_edge(d, f, "KNOWS").unwrap();
        graph.create_edge(f, d, "KNOWS").unwrap();

        let communities = label_propagation(&graph, 50);

        // クラスター1内のノードは同じコミュニティ
        let cluster1_label = communities[&a];
        assert_eq!(communities[&b], cluster1_label);
        assert_eq!(communities[&c], cluster1_label);

        // クラスター2内のノードは同じコミュニティ
        let cluster2_label = communities[&d];
        assert_eq!(communities[&e], cluster2_label);
        assert_eq!(communities[&f], cluster2_label);

        // 2つのクラスターは異なるコミュニティ
        assert_ne!(
            cluster1_label, cluster2_label,
            "2つのクラスターは分離されるべき"
        );
    }

    #[test]
    fn test_label_propagation_empty_graph() {
        let graph = Graph::new();
        let communities = label_propagation(&graph, 10);
        assert!(communities.is_empty());
    }

    #[test]
    fn test_label_propagation_single_node() {
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let communities = label_propagation(&graph, 10);
        assert_eq!(communities.len(), 1);
        assert_eq!(communities[&a], a);
    }
}
