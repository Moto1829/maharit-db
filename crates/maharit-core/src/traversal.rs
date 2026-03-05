use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

use crate::{Edge, EdgeId, Graph, Node, NodeId, PropertyValue};

/// トラバーサルの方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// 出るエッジを辿る
    Outgoing,
    /// 入るエッジを辿る
    Incoming,
    /// 両方向
    Both,
}

/// Type alias for edge filter closures used in traversal
type EdgeFilter<'a> = Option<Box<dyn Fn(&Edge) -> bool + 'a>>;
/// Type alias for node filter closures used in traversal
type NodeFilter<'a> = Option<Box<dyn Fn(&Node) -> bool + 'a>>;

/// トラバーサルビルダー
pub struct Traversal<'a> {
    graph: &'a Graph,
    start: NodeId,
    direction: Direction,
    max_depth: Option<usize>,
    edge_filter: EdgeFilter<'a>,
    node_filter: NodeFilter<'a>,
}

impl<'a> Traversal<'a> {
    pub fn new(graph: &'a Graph, start: NodeId) -> Self {
        Self {
            graph,
            start,
            direction: Direction::Outgoing,
            max_depth: None,
            edge_filter: None,
            node_filter: None,
        }
    }

    /// 探索方向を設定
    pub fn direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }

    /// 最大深さを設定
    pub fn max_depth(mut self, depth: usize) -> Self {
        self.max_depth = Some(depth);
        self
    }

    /// エッジのフィルタを設定
    pub fn with_edge_filter<F>(mut self, filter: F) -> Self
    where
        F: Fn(&Edge) -> bool + 'a,
    {
        self.edge_filter = Some(Box::new(filter));
        self
    }

    /// ノードのフィルタを設定
    pub fn with_node_filter<F>(mut self, filter: F) -> Self
    where
        F: Fn(&Node) -> bool + 'a,
    {
        self.node_filter = Some(Box::new(filter));
        self
    }

    /// BFS（幅優先探索）でイテレート
    pub fn bfs(self) -> BfsIterator<'a> {
        BfsIterator::new(self)
    }

    /// DFS（深さ優先探索）でイテレート
    pub fn dfs(self) -> DfsIterator<'a> {
        DfsIterator::new(self)
    }

    fn get_neighbors(&self, node_id: NodeId) -> Vec<(NodeId, &'a Edge)> {
        let mut neighbors = Vec::new();

        if self.direction == Direction::Outgoing || self.direction == Direction::Both {
            for edge in self.graph.get_outgoing_edges(node_id) {
                if self.edge_filter.as_ref().is_none_or(|f| f(edge)) {
                    neighbors.push((edge.to, edge));
                }
            }
        }

        if self.direction == Direction::Incoming || self.direction == Direction::Both {
            for edge in self.graph.get_incoming_edges(node_id) {
                if self.edge_filter.as_ref().is_none_or(|f| f(edge)) {
                    neighbors.push((edge.from, edge));
                }
            }
        }

        neighbors
    }

    fn should_visit_node(&self, node: &Node) -> bool {
        self.node_filter.as_ref().is_none_or(|f| f(node))
    }
}

/// BFSイテレータ
pub struct BfsIterator<'a> {
    traversal: Traversal<'a>,
    queue: VecDeque<(NodeId, usize)>, // (node_id, depth)
    visited: HashSet<NodeId>,
}

impl<'a> BfsIterator<'a> {
    fn new(traversal: Traversal<'a>) -> Self {
        let mut queue = VecDeque::new();
        let mut visited = HashSet::new();

        queue.push_back((traversal.start, 0));
        visited.insert(traversal.start);

        Self {
            traversal,
            queue,
            visited,
        }
    }
}

impl<'a> Iterator for BfsIterator<'a> {
    type Item = (NodeId, usize); // (node_id, depth)

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((node_id, depth)) = self.queue.pop_front() {
            // 深さ制限のチェック
            if let Some(max_depth) = self.traversal.max_depth
                && depth > max_depth
            {
                continue;
            }

            // ノードフィルタのチェック
            if let Some(node) = self.traversal.graph.get_node(node_id) {
                if !self.traversal.should_visit_node(node) {
                    continue;
                }
            } else {
                continue;
            }

            // 隣接ノードをキューに追加
            if self.traversal.max_depth.is_none_or(|max| depth < max) {
                for (neighbor_id, _edge) in self.traversal.get_neighbors(node_id) {
                    if !self.visited.contains(&neighbor_id) {
                        self.visited.insert(neighbor_id);
                        self.queue.push_back((neighbor_id, depth + 1));
                    }
                }
            }

            return Some((node_id, depth));
        }

        None
    }
}

/// DFSイテレータ
pub struct DfsIterator<'a> {
    traversal: Traversal<'a>,
    stack: Vec<(NodeId, usize)>, // (node_id, depth)
    visited: HashSet<NodeId>,
}

impl<'a> DfsIterator<'a> {
    fn new(traversal: Traversal<'a>) -> Self {
        let mut stack = Vec::new();
        let mut visited = HashSet::new();

        stack.push((traversal.start, 0));
        visited.insert(traversal.start);

        Self {
            traversal,
            stack,
            visited,
        }
    }
}

impl<'a> Iterator for DfsIterator<'a> {
    type Item = (NodeId, usize); // (node_id, depth)

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((node_id, depth)) = self.stack.pop() {
            // 深さ制限のチェック
            if let Some(max_depth) = self.traversal.max_depth
                && depth > max_depth
            {
                continue;
            }

            // ノードフィルタのチェック
            if let Some(node) = self.traversal.graph.get_node(node_id) {
                if !self.traversal.should_visit_node(node) {
                    continue;
                }
            } else {
                continue;
            }

            // 隣接ノードをスタックに追加
            if self.traversal.max_depth.is_none_or(|max| depth < max) {
                for (neighbor_id, _edge) in self.traversal.get_neighbors(node_id) {
                    if !self.visited.contains(&neighbor_id) {
                        self.visited.insert(neighbor_id);
                        self.stack.push((neighbor_id, depth + 1));
                    }
                }
            }

            return Some((node_id, depth));
        }

        None
    }
}

/// パス探索結果
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pub nodes: Vec<NodeId>,
}

impl Path {
    pub fn new(nodes: Vec<NodeId>) -> Self {
        Self { nodes }
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// 2ノード間の最短パス（ホップ数）を探索
pub fn shortest_path(graph: &Graph, from: NodeId, to: NodeId) -> Option<Path> {
    if from == to {
        return Some(Path::new(vec![from]));
    }

    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();
    let mut parent: std::collections::HashMap<NodeId, NodeId> = std::collections::HashMap::new();

    queue.push_back(from);
    visited.insert(from);

    while let Some(current) = queue.pop_front() {
        for edge in graph.get_outgoing_edges(current) {
            let neighbor = edge.to;
            if !visited.contains(&neighbor) {
                visited.insert(neighbor);
                parent.insert(neighbor, current);

                if neighbor == to {
                    // パスを復元
                    let mut path = vec![to];
                    let mut node = to;
                    while let Some(&p) = parent.get(&node) {
                        path.push(p);
                        node = p;
                    }
                    path.reverse();
                    return Some(Path::new(path));
                }

                queue.push_back(neighbor);
            }
        }
    }

    None
}

/// 2ノード間の全最短パスを探索
///
/// BFSで各ノードの全前任者(predecessors)を記録し、
/// ゴールから再帰的に全パスを復元する。
pub fn all_shortest_paths(graph: &Graph, from: NodeId, to: NodeId) -> Vec<Path> {
    if from == to {
        return vec![Path::new(vec![from])];
    }

    let mut queue = VecDeque::new();
    let mut distances: HashMap<NodeId, usize> = HashMap::new();
    let mut predecessors: HashMap<NodeId, Vec<NodeId>> = HashMap::new();

    queue.push_back(from);
    distances.insert(from, 0);

    let mut target_distance: Option<usize> = None;

    while let Some(current) = queue.pop_front() {
        let current_dist = distances[&current];

        // If we've found paths to target and current distance exceeds target distance, stop
        if let Some(td) = target_distance
            && current_dist >= td
        {
            continue;
        }

        for edge in graph.get_outgoing_edges(current) {
            let neighbor = edge.to;
            let new_dist = current_dist + 1;

            match distances.get(&neighbor) {
                None => {
                    // First time visiting this node
                    distances.insert(neighbor, new_dist);
                    predecessors.insert(neighbor, vec![current]);
                    queue.push_back(neighbor);

                    if neighbor == to {
                        target_distance = Some(new_dist);
                    }
                }
                Some(&existing_dist) if existing_dist == new_dist => {
                    // Same distance path found, add predecessor
                    predecessors.get_mut(&neighbor).unwrap().push(current);
                }
                _ => {
                    // Longer path, ignore
                }
            }
        }
    }

    // Reconstruct all paths from predecessors
    if !predecessors.contains_key(&to) && from != to {
        return vec![];
    }

    let mut all_paths = Vec::new();
    reconstruct_all_paths(from, to, &predecessors, &mut vec![to], &mut all_paths);
    all_paths
}

/// Helper function to recursively reconstruct all paths from predecessors
fn reconstruct_all_paths(
    from: NodeId,
    current: NodeId,
    predecessors: &HashMap<NodeId, Vec<NodeId>>,
    current_path: &mut Vec<NodeId>,
    all_paths: &mut Vec<Path>,
) {
    if current == from {
        let mut path = current_path.clone();
        path.reverse();
        all_paths.push(Path::new(path));
        return;
    }

    if let Some(preds) = predecessors.get(&current) {
        for &pred in preds {
            current_path.push(pred);
            reconstruct_all_paths(from, pred, predecessors, current_path, all_paths);
            current_path.pop();
        }
    }
}

/// 2ノード間の全単純パスを列挙（DFSバックトラッキング）
///
/// max_depth: 最大ホップ数の上限（Noneで無制限）
/// 注意: グラフが大きい場合は指数的に増加する可能性がある
pub fn all_paths(graph: &Graph, from: NodeId, to: NodeId, max_depth: Option<usize>) -> Vec<Path> {
    let mut results = Vec::new();
    if graph.get_node(from).is_none() || graph.get_node(to).is_none() {
        return results;
    }
    if from == to {
        return vec![Path::new(vec![from])];
    }
    let mut visited = HashSet::new();
    visited.insert(from);
    let mut current_path = vec![from];
    all_paths_dfs(
        graph,
        from,
        to,
        max_depth,
        0,
        &mut visited,
        &mut current_path,
        &mut results,
    );
    results
}

#[allow(clippy::too_many_arguments)]
fn all_paths_dfs(
    graph: &Graph,
    current: NodeId,
    to: NodeId,
    max_depth: Option<usize>,
    depth: usize,
    visited: &mut HashSet<NodeId>,
    current_path: &mut Vec<NodeId>,
    results: &mut Vec<Path>,
) {
    if let Some(max) = max_depth
        && depth >= max
    {
        return;
    }
    for edge in graph.get_outgoing_edges(current) {
        let neighbor = edge.to;
        if neighbor == to {
            let mut path = current_path.clone();
            path.push(to);
            results.push(Path::new(path));
        } else if !visited.contains(&neighbor) {
            visited.insert(neighbor);
            current_path.push(neighbor);
            all_paths_dfs(
                graph,
                neighbor,
                to,
                max_depth,
                depth + 1,
                visited,
                current_path,
                results,
            );
            current_path.pop();
            visited.remove(&neighbor);
        }
    }
}

/// 2ノード間にパスが存在するか確認
pub fn has_path(graph: &Graph, from: NodeId, to: NodeId) -> bool {
    if from == to {
        return true;
    }

    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();

    queue.push_back(from);
    visited.insert(from);

    while let Some(current) = queue.pop_front() {
        for edge in graph.get_outgoing_edges(current) {
            let neighbor = edge.to;
            if neighbor == to {
                return true;
            }
            if !visited.contains(&neighbor) {
                visited.insert(neighbor);
                queue.push_back(neighbor);
            }
        }
    }

    false
}

// ========== 重み付き最短経路 ==========

/// 重み付き最短経路の結果
#[derive(Debug, Clone, PartialEq)]
pub struct WeightedPath {
    pub nodes: Vec<NodeId>,
    pub edges: Vec<EdgeId>,
    pub total_weight: f64,
}

impl WeightedPath {
    pub fn new(nodes: Vec<NodeId>, edges: Vec<EdgeId>, total_weight: f64) -> Self {
        Self {
            nodes,
            edges,
            total_weight,
        }
    }
}

/// Dijkstraのヒープエントリ
#[derive(Debug, Clone)]
struct DijkstraEntry {
    node: NodeId,
    distance: f64,
}

impl PartialEq for DijkstraEntry {
    fn eq(&self, other: &Self) -> bool {
        self.distance == other.distance
    }
}

impl Eq for DijkstraEntry {}

impl Ord for DijkstraEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // 最小ヒープにするため逆順
        other
            .distance
            .partial_cmp(&self.distance)
            .unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for DijkstraEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 重み取得関数の型
pub type WeightFn<'a> = Box<dyn Fn(&Edge) -> f64 + 'a>;

/// Dijkstra法による最短経路探索
pub struct Dijkstra<'a> {
    graph: &'a Graph,
    weight_fn: WeightFn<'a>,
}

impl<'a> Dijkstra<'a> {
    /// デフォルトの重み（すべて1.0）で作成
    pub fn new(graph: &'a Graph) -> Self {
        Self {
            graph,
            weight_fn: Box::new(|_| 1.0),
        }
    }

    /// エッジプロパティから重みを取得
    pub fn with_weight_property(graph: &'a Graph, property: &'a str) -> Self {
        Self {
            graph,
            weight_fn: Box::new(move |edge| {
                edge.get_property(property)
                    .and_then(|v| match v {
                        PropertyValue::Int(n) => Some(*n as f64),
                        PropertyValue::Float(n) => Some(*n),
                        _ => None,
                    })
                    .unwrap_or(1.0)
            }),
        }
    }

    /// カスタム重み関数を使用
    pub fn with_weight_fn<F>(graph: &'a Graph, weight_fn: F) -> Self
    where
        F: Fn(&Edge) -> f64 + 'a,
    {
        Self {
            graph,
            weight_fn: Box::new(weight_fn),
        }
    }

    /// 単一始点から特定の終点への最短経路
    pub fn find_path(&self, from: NodeId, to: NodeId) -> Option<WeightedPath> {
        if from == to {
            return Some(WeightedPath::new(vec![from], vec![], 0.0));
        }

        let mut distances: HashMap<NodeId, f64> = HashMap::new();
        let mut previous: HashMap<NodeId, (NodeId, EdgeId)> = HashMap::new();
        let mut heap = BinaryHeap::new();

        distances.insert(from, 0.0);
        heap.push(DijkstraEntry {
            node: from,
            distance: 0.0,
        });

        while let Some(DijkstraEntry { node, distance }) = heap.pop() {
            if node == to {
                return Some(self.reconstruct_path(from, to, &previous, distance));
            }

            // すでにより短い経路が見つかっている場合はスキップ
            if let Some(&d) = distances.get(&node)
                && distance > d
            {
                continue;
            }

            for edge in self.graph.get_outgoing_edges(node) {
                let weight = (self.weight_fn)(edge);
                let next_distance = distance + weight;
                let neighbor = edge.to;

                if !distances.contains_key(&neighbor) || next_distance < distances[&neighbor] {
                    distances.insert(neighbor, next_distance);
                    previous.insert(neighbor, (node, edge.id));
                    heap.push(DijkstraEntry {
                        node: neighbor,
                        distance: next_distance,
                    });
                }
            }
        }

        None
    }

    /// 単一始点から全ノードへの最短距離
    pub fn distances_from(&self, from: NodeId) -> HashMap<NodeId, f64> {
        let mut distances: HashMap<NodeId, f64> = HashMap::new();
        let mut heap = BinaryHeap::new();

        distances.insert(from, 0.0);
        heap.push(DijkstraEntry {
            node: from,
            distance: 0.0,
        });

        while let Some(DijkstraEntry { node, distance }) = heap.pop() {
            if let Some(&d) = distances.get(&node)
                && distance > d
            {
                continue;
            }

            for edge in self.graph.get_outgoing_edges(node) {
                let weight = (self.weight_fn)(edge);
                let next_distance = distance + weight;
                let neighbor = edge.to;

                if !distances.contains_key(&neighbor) || next_distance < distances[&neighbor] {
                    distances.insert(neighbor, next_distance);
                    heap.push(DijkstraEntry {
                        node: neighbor,
                        distance: next_distance,
                    });
                }
            }
        }

        distances
    }

    fn reconstruct_path(
        &self,
        from: NodeId,
        to: NodeId,
        previous: &HashMap<NodeId, (NodeId, EdgeId)>,
        total_weight: f64,
    ) -> WeightedPath {
        let mut nodes = vec![to];
        let mut edges = Vec::new();
        let mut current = to;

        while current != from {
            if let Some(&(prev_node, edge_id)) = previous.get(&current) {
                nodes.push(prev_node);
                edges.push(edge_id);
                current = prev_node;
            } else {
                break;
            }
        }

        nodes.reverse();
        edges.reverse();

        WeightedPath::new(nodes, edges, total_weight)
    }
}

/// A*アルゴリズム用のヒューリスティック関数
pub type HeuristicFn<'a> = Box<dyn Fn(NodeId, NodeId) -> f64 + 'a>;

/// A*のヒープエントリ
#[derive(Debug, Clone)]
struct AStarEntry {
    node: NodeId,
    g_score: f64, // 実際のコスト
    f_score: f64, // g + h
}

impl PartialEq for AStarEntry {
    fn eq(&self, other: &Self) -> bool {
        self.f_score == other.f_score
    }
}

impl Eq for AStarEntry {}

impl Ord for AStarEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .f_score
            .partial_cmp(&self.f_score)
            .unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for AStarEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A*アルゴリズムによる最短経路探索
pub struct AStar<'a> {
    graph: &'a Graph,
    weight_fn: WeightFn<'a>,
    heuristic: HeuristicFn<'a>,
}

impl<'a> AStar<'a> {
    /// ヒューリスティック関数を指定して作成
    pub fn new<H>(graph: &'a Graph, heuristic: H) -> Self
    where
        H: Fn(NodeId, NodeId) -> f64 + 'a,
    {
        Self {
            graph,
            weight_fn: Box::new(|_| 1.0),
            heuristic: Box::new(heuristic),
        }
    }

    /// 重み関数も指定
    pub fn with_weight_fn<W, H>(graph: &'a Graph, weight_fn: W, heuristic: H) -> Self
    where
        W: Fn(&Edge) -> f64 + 'a,
        H: Fn(NodeId, NodeId) -> f64 + 'a,
    {
        Self {
            graph,
            weight_fn: Box::new(weight_fn),
            heuristic: Box::new(heuristic),
        }
    }

    /// A*で最短経路を探索
    pub fn find_path(&self, from: NodeId, to: NodeId) -> Option<WeightedPath> {
        if from == to {
            return Some(WeightedPath::new(vec![from], vec![], 0.0));
        }

        let mut g_scores: HashMap<NodeId, f64> = HashMap::new();
        let mut previous: HashMap<NodeId, (NodeId, EdgeId)> = HashMap::new();
        let mut heap = BinaryHeap::new();
        let mut closed: HashSet<NodeId> = HashSet::new();

        g_scores.insert(from, 0.0);
        heap.push(AStarEntry {
            node: from,
            g_score: 0.0,
            f_score: (self.heuristic)(from, to),
        });

        while let Some(AStarEntry { node, g_score, .. }) = heap.pop() {
            if node == to {
                return Some(self.reconstruct_path(from, to, &previous, g_score));
            }

            if closed.contains(&node) {
                continue;
            }
            closed.insert(node);

            for edge in self.graph.get_outgoing_edges(node) {
                let neighbor = edge.to;
                if closed.contains(&neighbor) {
                    continue;
                }

                let weight = (self.weight_fn)(edge);
                let tentative_g = g_score + weight;

                if !g_scores.contains_key(&neighbor) || tentative_g < g_scores[&neighbor] {
                    g_scores.insert(neighbor, tentative_g);
                    previous.insert(neighbor, (node, edge.id));

                    let f_score = tentative_g + (self.heuristic)(neighbor, to);
                    heap.push(AStarEntry {
                        node: neighbor,
                        g_score: tentative_g,
                        f_score,
                    });
                }
            }
        }

        None
    }

    fn reconstruct_path(
        &self,
        from: NodeId,
        to: NodeId,
        previous: &HashMap<NodeId, (NodeId, EdgeId)>,
        total_weight: f64,
    ) -> WeightedPath {
        let mut nodes = vec![to];
        let mut edges = Vec::new();
        let mut current = to;

        while current != from {
            if let Some(&(prev_node, edge_id)) = previous.get(&current) {
                nodes.push(prev_node);
                edges.push(edge_id);
                current = prev_node;
            } else {
                break;
            }
        }

        nodes.reverse();
        edges.reverse();

        WeightedPath::new(nodes, edges, total_weight)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_graph() -> Graph {
        // A -> B -> C
        //      |
        //      v
        //      D -> E
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        let c = graph.create_node("C");
        let d = graph.create_node("D");
        let e = graph.create_node("E");

        graph.create_edge(a, b, "NEXT").unwrap();
        graph.create_edge(b, c, "NEXT").unwrap();
        graph.create_edge(b, d, "BRANCH").unwrap();
        graph.create_edge(d, e, "NEXT").unwrap();

        graph
    }

    #[test]
    fn test_bfs() {
        let graph = create_test_graph();
        let nodes: Vec<NodeId> = Traversal::new(&graph, 0).bfs().map(|(id, _)| id).collect();

        assert_eq!(nodes.len(), 5);
        assert_eq!(nodes[0], 0); // A (depth 0)
        assert_eq!(nodes[1], 1); // B (depth 1)
        // C, D are at depth 2
        assert!(nodes[2..4].contains(&2));
        assert!(nodes[2..4].contains(&3));
        assert_eq!(nodes[4], 4); // E (depth 3)
    }

    #[test]
    fn test_dfs() {
        let graph = create_test_graph();
        let nodes: Vec<NodeId> = Traversal::new(&graph, 0).dfs().map(|(id, _)| id).collect();

        assert_eq!(nodes.len(), 5);
        assert_eq!(nodes[0], 0); // Start from A
    }

    #[test]
    fn test_max_depth() {
        let graph = create_test_graph();
        let nodes: Vec<NodeId> = Traversal::new(&graph, 0)
            .max_depth(1)
            .bfs()
            .map(|(id, _)| id)
            .collect();

        assert_eq!(nodes.len(), 2); // A, B only
        assert!(nodes.contains(&0));
        assert!(nodes.contains(&1));
    }

    #[test]
    fn test_edge_filter() {
        let graph = create_test_graph();
        let nodes: Vec<NodeId> = Traversal::new(&graph, 0)
            .with_edge_filter(|e| e.label == "NEXT")
            .bfs()
            .map(|(id, _)| id)
            .collect();

        // A -> B -> C (only NEXT edges)
        assert_eq!(nodes.len(), 3);
        assert!(nodes.contains(&0));
        assert!(nodes.contains(&1));
        assert!(nodes.contains(&2));
        assert!(!nodes.contains(&3)); // D is via BRANCH
    }

    #[test]
    fn test_shortest_path() {
        let graph = create_test_graph();

        let path = shortest_path(&graph, 0, 4).unwrap();
        assert_eq!(path.nodes, vec![0, 1, 3, 4]); // A -> B -> D -> E

        let path = shortest_path(&graph, 0, 2).unwrap();
        assert_eq!(path.nodes, vec![0, 1, 2]); // A -> B -> C
    }

    #[test]
    fn test_shortest_path_same_node() {
        let graph = create_test_graph();
        let path = shortest_path(&graph, 0, 0).unwrap();
        assert_eq!(path.nodes, vec![0]);
    }

    #[test]
    fn test_shortest_path_no_path() {
        let graph = create_test_graph();
        // E has no outgoing edges, so no path from E to A
        let path = shortest_path(&graph, 4, 0);
        assert!(path.is_none());
    }

    #[test]
    fn test_has_path() {
        let graph = create_test_graph();

        assert!(has_path(&graph, 0, 4)); // A to E
        assert!(has_path(&graph, 0, 0)); // Same node
        assert!(!has_path(&graph, 4, 0)); // E to A (no reverse edges)
    }

    #[test]
    fn test_bidirectional() {
        let graph = create_test_graph();
        let nodes: Vec<NodeId> = Traversal::new(&graph, 1) // Start from B
            .direction(Direction::Both)
            .max_depth(1)
            .bfs()
            .map(|(id, _)| id)
            .collect();

        // B has incoming from A, outgoing to C and D
        assert!(nodes.contains(&1)); // B
        assert!(nodes.contains(&0)); // A (incoming)
        assert!(nodes.contains(&2)); // C (outgoing)
        assert!(nodes.contains(&3)); // D (outgoing)
    }

    // ========== Dijkstra tests ==========

    fn create_weighted_graph() -> Graph {
        // A --2--> B --3--> C
        //  \              /
        //   \--10------->/
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        let c = graph.create_node("C");

        let e1 = graph.create_edge(a, b, "road").unwrap();
        let e2 = graph.create_edge(b, c, "road").unwrap();
        let e3 = graph.create_edge(a, c, "highway").unwrap();

        // Set weights
        if let Some(edge) = graph.get_edge_mut(e1) {
            edge.set_property("weight", 2.0);
        }
        if let Some(edge) = graph.get_edge_mut(e2) {
            edge.set_property("weight", 3.0);
        }
        if let Some(edge) = graph.get_edge_mut(e3) {
            edge.set_property("weight", 10.0);
        }

        graph
    }

    #[test]
    fn test_dijkstra_unweighted() {
        let graph = create_test_graph();
        let dijkstra = Dijkstra::new(&graph);

        let path = dijkstra.find_path(0, 4).unwrap();
        assert_eq!(path.nodes, vec![0, 1, 3, 4]);
        assert_eq!(path.total_weight, 3.0); // 3 hops
    }

    #[test]
    fn test_dijkstra_weighted() {
        let graph = create_weighted_graph();
        let dijkstra = Dijkstra::with_weight_property(&graph, "weight");

        let path = dijkstra.find_path(0, 2).unwrap();
        // A -> B -> C (2 + 3 = 5) is shorter than A -> C (10)
        assert_eq!(path.nodes, vec![0, 1, 2]);
        assert_eq!(path.total_weight, 5.0);
    }

    #[test]
    fn test_dijkstra_distances_from() {
        let graph = create_weighted_graph();
        let dijkstra = Dijkstra::with_weight_property(&graph, "weight");

        let distances = dijkstra.distances_from(0);

        assert_eq!(distances.get(&0), Some(&0.0));
        assert_eq!(distances.get(&1), Some(&2.0));
        assert_eq!(distances.get(&2), Some(&5.0)); // Via B, not direct
    }

    #[test]
    fn test_dijkstra_no_path() {
        let graph = create_test_graph();
        let dijkstra = Dijkstra::new(&graph);

        let path = dijkstra.find_path(4, 0); // E to A, no path
        assert!(path.is_none());
    }

    #[test]
    fn test_dijkstra_same_node() {
        let graph = create_test_graph();
        let dijkstra = Dijkstra::new(&graph);

        let path = dijkstra.find_path(0, 0).unwrap();
        assert_eq!(path.nodes, vec![0]);
        assert_eq!(path.total_weight, 0.0);
    }

    // ========== A* tests ==========

    #[test]
    fn test_astar_basic() {
        let graph = create_test_graph();
        // Simple heuristic: always 0 (becomes Dijkstra)
        let astar = AStar::new(&graph, |_, _| 0.0);

        let path = astar.find_path(0, 4).unwrap();
        assert_eq!(path.nodes, vec![0, 1, 3, 4]);
    }

    #[test]
    fn test_astar_with_heuristic() {
        let graph = create_weighted_graph();
        // Heuristic based on node distance (simplified)
        let astar = AStar::with_weight_fn(
            &graph,
            |edge| {
                edge.get_property("weight")
                    .and_then(|v| match v {
                        PropertyValue::Float(n) => Some(*n),
                        _ => None,
                    })
                    .unwrap_or(1.0)
            },
            |current, goal| {
                // Admissible heuristic
                if current == goal { 0.0 } else { 1.0 }
            },
        );

        let path = astar.find_path(0, 2).unwrap();
        assert_eq!(path.nodes, vec![0, 1, 2]);
        assert_eq!(path.total_weight, 5.0);
    }

    #[test]
    fn test_astar_same_node() {
        let graph = create_test_graph();
        let astar = AStar::new(&graph, |_, _| 0.0);

        let path = astar.find_path(0, 0).unwrap();
        assert_eq!(path.nodes, vec![0]);
        assert_eq!(path.total_weight, 0.0);
    }

    // ========== all_shortest_paths tests ==========

    #[test]
    fn test_all_shortest_paths_single() {
        let graph = create_test_graph();
        // A -> B -> C: only one shortest path
        let paths = all_shortest_paths(&graph, 0, 2);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].nodes, vec![0, 1, 2]);
    }

    #[test]
    fn test_all_shortest_paths_multiple() {
        // Create a diamond graph: A -> B -> D, A -> C -> D
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        let c = graph.create_node("C");
        let d = graph.create_node("D");

        graph.create_edge(a, b, "E1").unwrap();
        graph.create_edge(a, c, "E2").unwrap();
        graph.create_edge(b, d, "E3").unwrap();
        graph.create_edge(c, d, "E4").unwrap();

        let paths = all_shortest_paths(&graph, a, d);
        assert_eq!(paths.len(), 2);

        // Both paths should have length 3 (A, middle, D)
        for path in &paths {
            assert_eq!(path.nodes.len(), 3);
            assert_eq!(path.nodes[0], a);
            assert_eq!(path.nodes[2], d);
        }

        // The middle nodes should be B and C (in some order)
        let middle_nodes: Vec<NodeId> = paths.iter().map(|p| p.nodes[1]).collect();
        assert!(middle_nodes.contains(&b));
        assert!(middle_nodes.contains(&c));
    }

    #[test]
    fn test_all_shortest_paths_same_node() {
        let graph = create_test_graph();
        let paths = all_shortest_paths(&graph, 0, 0);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].nodes, vec![0]);
    }

    #[test]
    fn test_all_shortest_paths_no_path() {
        let graph = create_test_graph();
        // E has no outgoing edges, so no path from E to A
        let paths = all_shortest_paths(&graph, 4, 0);
        assert!(paths.is_empty());
    }

    #[test]
    fn test_all_paths_basic() {
        // A -> B -> C
        //      |
        //      v
        //      D -> C  (追加: D -> C)
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        let c = graph.create_node("C");
        let d = graph.create_node("D");
        graph.create_edge(a, b, "E");
        graph.create_edge(b, c, "E");
        graph.create_edge(b, d, "E");
        graph.create_edge(d, c, "E");

        let paths = all_paths(&graph, a, c, None);
        assert_eq!(paths.len(), 2);
        // A->B->C and A->B->D->C
        let path_nodes: Vec<Vec<NodeId>> = paths.iter().map(|p| p.nodes.clone()).collect();
        assert!(path_nodes.contains(&vec![a, b, c]));
        assert!(path_nodes.contains(&vec![a, b, d, c]));
    }

    #[test]
    fn test_all_paths_same_node() {
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let paths = all_paths(&graph, a, a, None);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].nodes, vec![a]);
    }

    #[test]
    fn test_all_paths_no_path() {
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        // no edge
        let paths = all_paths(&graph, a, b, None);
        assert!(paths.is_empty());
    }

    #[test]
    fn test_all_paths_max_depth() {
        // A -> B -> C -> D
        // A -> D (direct shortcut)
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        let c = graph.create_node("C");
        let d = graph.create_node("D");
        graph.create_edge(a, b, "E");
        graph.create_edge(b, c, "E");
        graph.create_edge(c, d, "E");
        graph.create_edge(a, d, "E");

        // max_depth=1: only A->D (1 hop)
        let paths = all_paths(&graph, a, d, Some(1));
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].nodes, vec![a, d]);

        // max_depth=3: A->D and A->B->C->D
        let paths = all_paths(&graph, a, d, Some(3));
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn test_all_paths_avoids_cycles() {
        // A -> B -> A (cycle) + B -> C
        let mut graph = Graph::new();
        let a = graph.create_node("A");
        let b = graph.create_node("B");
        let c = graph.create_node("C");
        graph.create_edge(a, b, "E");
        graph.create_edge(b, a, "E"); // cycle
        graph.create_edge(b, c, "E");

        // Should find A->B->C and not loop infinitely
        let paths = all_paths(&graph, a, c, None);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].nodes, vec![a, b, c]);
    }
}
