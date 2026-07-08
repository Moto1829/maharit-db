use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::traversal::Traversal;
use crate::{GraphError, PropertyValue};

pub type NodeId = u64;
pub type EdgeId = u64;

/// ノード（頂点）を表す構造体
#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    /// ノードが持つラベルのリスト（複数ラベル対応）
    pub labels: Vec<String>,
    pub properties: Arc<HashMap<String, PropertyValue>>,
}

impl Node {
    /// 最初のラベルを返す後方互換ヘルパー。
    /// ラベルがない場合は空文字列を返す。
    pub fn primary_label(&self) -> &str {
        self.labels.first().map(|s| s.as_str()).unwrap_or("")
    }

    /// 指定したラベルをノードが持つかを確認する
    pub fn has_label(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l == label)
    }

    /// ラベルを追加する（重複は無視）
    pub fn add_label(&mut self, label: impl Into<String>) {
        let label = label.into();
        if !label.is_empty() && !self.labels.contains(&label) {
            self.labels.push(label);
        }
    }

    /// ラベルを削除する
    pub fn remove_label(&mut self, label: &str) {
        self.labels.retain(|l| l != label);
    }

    /// プロパティを設定
    pub fn set_property(&mut self, key: impl Into<String>, value: impl Into<PropertyValue>) {
        Arc::make_mut(&mut self.properties).insert(key.into(), value.into());
    }

    /// プロパティを取得
    pub fn get_property(&self, key: &str) -> Option<&PropertyValue> {
        self.properties.get(key)
    }

    /// プロパティを削除
    pub fn remove_property(&mut self, key: &str) -> Option<PropertyValue> {
        Arc::make_mut(&mut self.properties).remove(key)
    }
}

/// エッジ（辺）を表す構造体
#[derive(Debug, Clone)]
pub struct Edge {
    pub id: EdgeId,
    pub label: String,
    pub from: NodeId,
    pub to: NodeId,
    pub properties: Arc<HashMap<String, PropertyValue>>,
}

impl Edge {
    /// プロパティを設定
    pub fn set_property(&mut self, key: impl Into<String>, value: impl Into<PropertyValue>) {
        Arc::make_mut(&mut self.properties).insert(key.into(), value.into());
    }

    /// プロパティを取得
    pub fn get_property(&self, key: &str) -> Option<&PropertyValue> {
        self.properties.get(key)
    }

    /// プロパティを削除
    pub fn remove_property(&mut self, key: &str) -> Option<PropertyValue> {
        Arc::make_mut(&mut self.properties).remove(key)
    }
}

/// インメモリグラフデータベース
///
/// ノード/エッジは連番 `u64` IDで管理されるため、`HashMap` の代わりに
/// `Vec<Option<T>>` を使って O(1) の直接アクセスを実現する。
/// 削除済みスロットはフリーリストに追加し再利用する。
#[derive(Debug, Default)]
pub struct Graph {
    /// NodeId をインデックスとするノード配列（削除済みは None）
    nodes: Vec<Option<Node>>,
    /// EdgeId をインデックスとするエッジ配列（削除済みは None）
    edges: Vec<Option<Edge>>,
    /// ノードから出るエッジのリスト（NodeId をインデックス）
    outgoing_edges: Vec<HashSet<EdgeId>>,
    /// ノードに入るエッジのリスト（NodeId をインデックス）
    incoming_edges: Vec<HashSet<EdgeId>>,
    /// 削除済みノードスロットの再利用リスト
    node_free_list: Vec<NodeId>,
    /// 削除済みエッジスロットの再利用リスト
    edge_free_list: Vec<EdgeId>,
    /// 存在するノードの数（None スロットを除く）
    node_count: usize,
    /// 存在するエッジの数（None スロットを除く）
    edge_count: usize,
    /// ラベル → そのラベルを持つ NodeId 集合。`MATCH (n:Label)` を全走査せずに
    /// 該当ノードのみ列挙するための索引。ノード作成/削除/ラベル増減で維持する。
    /// 永続化はロード時に create 経由で再構築されるためシリアライズ対象外。
    label_index: HashMap<String, HashSet<NodeId>>,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    // ── ラベル索引の維持 ─────────────────────────────────────────────────────

    /// 指定ノードの現在のラベルを索引に登録する（作成時に呼ぶ）。
    fn label_index_add_node(&mut self, id: NodeId) {
        let labels: Vec<String> = match self.nodes.get(id as usize).and_then(|s| s.as_ref()) {
            Some(n) => n.labels.clone(),
            None => return,
        };
        for label in labels {
            self.label_index.entry(label).or_default().insert(id);
        }
    }

    /// 指定ラベルの索引から `id` を除去する。空になったエントリは削除する。
    fn label_index_remove(&mut self, id: NodeId, label: &str) {
        if let Some(set) = self.label_index.get_mut(label) {
            set.remove(&id);
            if set.is_empty() {
                self.label_index.remove(label);
            }
        }
    }

    /// 指定ラベルを持つノード ID を返す（索引参照、全走査なし）。
    pub fn nodes_by_label(&self, label: &str) -> Vec<NodeId> {
        self.label_index
            .get(label)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// ノードにラベルを追加し、索引も更新する。
    pub fn add_node_label(&mut self, id: NodeId, label: String) {
        let added = if let Some(Some(node)) = self.nodes.get_mut(id as usize) {
            if node.has_label(&label) {
                false
            } else {
                node.add_label(label.clone());
                true
            }
        } else {
            false
        };
        if added {
            self.label_index.entry(label).or_default().insert(id);
        }
    }

    /// ノードからラベルを削除し、索引も更新する。
    pub fn remove_node_label(&mut self, id: NodeId, label: &str) {
        let removed = if let Some(Some(node)) = self.nodes.get_mut(id as usize) {
            let had = node.has_label(label);
            if had {
                node.remove_label(label);
            }
            had
        } else {
            false
        };
        if removed {
            self.label_index_remove(id, label);
        }
    }

    /// 新しいノードを作成して追加（単一ラベル版、後方互換のため残す）
    pub fn create_node(&mut self, label: impl Into<String>) -> NodeId {
        let label_str = label.into();
        let labels = if label_str.is_empty() {
            vec![]
        } else {
            vec![label_str]
        };
        self.create_node_with_labels(labels)
    }

    /// 複数ラベルを指定してノードを作成する
    pub fn create_node_with_labels(&mut self, labels: Vec<String>) -> NodeId {
        let id = if let Some(freed) = self.node_free_list.pop() {
            // Reuse a deleted slot
            self.nodes[freed as usize] = Some(Node {
                id: freed,
                labels,
                properties: Arc::new(HashMap::new()),
            });
            self.outgoing_edges[freed as usize] = HashSet::new();
            self.incoming_edges[freed as usize] = HashSet::new();
            freed
        } else {
            // Append a new slot
            let id = self.nodes.len() as NodeId;
            self.nodes.push(Some(Node {
                id,
                labels,
                properties: Arc::new(HashMap::new()),
            }));
            self.outgoing_edges.push(HashSet::new());
            self.incoming_edges.push(HashSet::new());
            id
        };
        self.node_count += 1;
        self.label_index_add_node(id);
        id
    }

    /// 指定したIDでノードを作成する（バックアップ/リストア専用、単一ラベル版）
    ///
    /// 指定した `id` が既に使用されている場合は既存ノードの ID を返す。
    pub fn create_node_with_id(&mut self, id: NodeId, label: impl Into<String>) -> NodeId {
        let label_str = label.into();
        let labels = if label_str.is_empty() {
            vec![]
        } else {
            vec![label_str]
        };
        self.create_node_with_id_and_labels(id, labels)
    }

    /// 指定したIDと複数ラベルでノードを作成する（バックアップ/リストア専用）
    pub fn create_node_with_id_and_labels(&mut self, id: NodeId, labels: Vec<String>) -> NodeId {
        // Already exists?
        if matches!(self.nodes.get(id as usize), Some(Some(_))) {
            return id;
        }

        // Extend backing vecs if needed
        let idx = id as usize;
        if idx >= self.nodes.len() {
            let new_len = idx + 1;
            self.nodes.resize_with(new_len, || None);
            self.outgoing_edges.resize_with(new_len, HashSet::new);
            self.incoming_edges.resize_with(new_len, HashSet::new);
        }

        // Remove from free list if it was there (slot is being explicitly assigned)
        self.node_free_list.retain(|&fid| fid != id);

        self.nodes[idx] = Some(Node {
            id,
            labels,
            properties: Arc::new(HashMap::new()),
        });
        self.outgoing_edges[idx] = HashSet::new();
        self.incoming_edges[idx] = HashSet::new();
        self.node_count += 1;
        self.label_index_add_node(id);
        id
    }

    /// ノードを取得
    pub fn get_node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id as usize)?.as_ref()
    }

    /// ノードを可変参照で取得
    pub fn get_node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id as usize)?.as_mut()
    }

    /// ノードを削除
    pub fn delete_node(&mut self, id: NodeId) -> Option<Node> {
        // Not present?
        if !matches!(self.nodes.get(id as usize), Some(Some(_))) {
            return None;
        }

        // Remove edges going FROM this node
        let outgoing = std::mem::take(&mut self.outgoing_edges[id as usize]);
        for edge_id in outgoing {
            if let Some(slot @ Some(_)) = self.edges.get_mut(edge_id as usize) {
                let to = slot.as_ref().unwrap().to;
                slot.take();
                if (to as usize) < self.incoming_edges.len() {
                    self.incoming_edges[to as usize].remove(&edge_id);
                }
                self.edge_free_list.push(edge_id);
                self.edge_count -= 1;
            }
        }

        // Remove edges going TO this node
        let incoming = std::mem::take(&mut self.incoming_edges[id as usize]);
        for edge_id in incoming {
            if let Some(slot @ Some(_)) = self.edges.get_mut(edge_id as usize) {
                let from = slot.as_ref().unwrap().from;
                slot.take();
                if (from as usize) < self.outgoing_edges.len() {
                    self.outgoing_edges[from as usize].remove(&edge_id);
                }
                self.edge_free_list.push(edge_id);
                self.edge_count -= 1;
            }
        }

        let node = self.nodes[id as usize].take();
        if let Some(ref n) = node {
            // ラベル索引から除去
            let labels: Vec<String> = n.labels.clone();
            for label in &labels {
                self.label_index_remove(id, label);
            }
            self.node_free_list.push(id);
            self.node_count -= 1;
        }
        node
    }

    /// 新しいエッジを作成して追加
    pub fn create_edge(
        &mut self,
        from: NodeId,
        to: NodeId,
        label: impl Into<String>,
    ) -> Result<EdgeId, GraphError> {
        if !matches!(self.nodes.get(from as usize), Some(Some(_))) {
            return Err(GraphError::NodeNotFound(from));
        }
        if !matches!(self.nodes.get(to as usize), Some(Some(_))) {
            return Err(GraphError::NodeNotFound(to));
        }

        let id = if let Some(freed) = self.edge_free_list.pop() {
            self.edges[freed as usize] = Some(Edge {
                id: freed,
                label: label.into(),
                from,
                to,
                properties: Arc::new(HashMap::new()),
            });
            freed
        } else {
            let id = self.edges.len() as EdgeId;
            self.edges.push(Some(Edge {
                id,
                label: label.into(),
                from,
                to,
                properties: Arc::new(HashMap::new()),
            }));
            id
        };

        self.outgoing_edges[from as usize].insert(id);
        self.incoming_edges[to as usize].insert(id);
        self.edge_count += 1;

        Ok(id)
    }

    /// エッジを取得
    pub fn get_edge(&self, id: EdgeId) -> Option<&Edge> {
        self.edges.get(id as usize)?.as_ref()
    }

    /// エッジを可変参照で取得
    pub fn get_edge_mut(&mut self, id: EdgeId) -> Option<&mut Edge> {
        self.edges.get_mut(id as usize)?.as_mut()
    }

    /// エッジを削除
    pub fn delete_edge(&mut self, id: EdgeId) -> Option<Edge> {
        let slot = self.edges.get_mut(id as usize)?;
        let edge = slot.take()?;
        if (edge.from as usize) < self.outgoing_edges.len() {
            self.outgoing_edges[edge.from as usize].remove(&id);
        }
        if (edge.to as usize) < self.incoming_edges.len() {
            self.incoming_edges[edge.to as usize].remove(&id);
        }
        self.edge_free_list.push(id);
        self.edge_count -= 1;
        Some(edge)
    }

    /// ノードから出るエッジをイテレートする（アロケーションなし）
    pub fn get_outgoing_edges(&self, node_id: NodeId) -> impl Iterator<Item = &Edge> + '_ {
        self.outgoing_edges
            .get(node_id as usize)
            .into_iter()
            .flat_map(|set| set.iter().copied())
            .filter_map(|id| self.edges.get(id as usize)?.as_ref())
    }

    /// ノードに入るエッジをイテレートする（アロケーションなし）
    pub fn get_incoming_edges(&self, node_id: NodeId) -> impl Iterator<Item = &Edge> + '_ {
        self.incoming_edges
            .get(node_id as usize)
            .into_iter()
            .flat_map(|set| set.iter().copied())
            .filter_map(|id| self.edges.get(id as usize)?.as_ref())
    }

    /// NodeId の上限（稠密インデックス用）。現在の Vec キャパシティに等しい。
    pub fn node_capacity(&self) -> usize {
        self.nodes.len()
    }

    /// EdgeId の上限（稠密インデックス用）
    pub fn edge_capacity(&self) -> usize {
        self.edges.len()
    }

    /// 隣接ノードを取得（出るエッジの先）
    pub fn get_neighbors(&self, node_id: NodeId) -> Vec<&Node> {
        self.get_outgoing_edges(node_id)
            .filter_map(|edge| self.get_node(edge.to))
            .collect()
    }

    /// 全ノード数
    pub fn node_count(&self) -> usize {
        self.node_count
    }

    /// 全エッジ数
    pub fn edge_count(&self) -> usize {
        self.edge_count
    }

    /// 全ノードをイテレート（削除済みスロットをスキップ）
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter_map(|n| n.as_ref())
    }

    /// 全エッジをイテレート（削除済みスロットをスキップ）
    pub fn edges(&self) -> impl Iterator<Item = &Edge> {
        self.edges.iter().filter_map(|e| e.as_ref())
    }

    /// 指定したノードからトラバーサルを開始
    pub fn traverse(&self, start: NodeId) -> Traversal<'_> {
        Traversal::new(self, start)
    }

    /// ラベルでノードを検索（指定ラベルを持つノードを全て返す）
    pub fn find_nodes_by_label(&self, label: &str) -> Vec<&Node> {
        self.nodes()
            .filter(|n| n.has_label(label))
            .collect()
    }

    /// ラベル（タイプ）でエッジを検索
    pub fn find_edges_by_type(&self, edge_type: &str) -> Vec<&Edge> {
        self.edges()
            .filter(|e| e.label == edge_type)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut v: Vec<NodeId>) -> Vec<NodeId> {
        v.sort_unstable();
        v
    }

    #[test]
    fn test_label_index_create_and_lookup() {
        let mut graph = Graph::new();
        let a = graph.create_node("Person");
        let b = graph.create_node("Person");
        let c = graph.create_node("City");

        assert_eq!(sorted(graph.nodes_by_label("Person")), vec![a, b]);
        assert_eq!(graph.nodes_by_label("City"), vec![c]);
        assert!(graph.nodes_by_label("Nonexistent").is_empty());
    }

    #[test]
    fn test_label_index_delete_consistency() {
        let mut graph = Graph::new();
        let a = graph.create_node("Person");
        let b = graph.create_node("Person");
        graph.delete_node(a);

        assert_eq!(graph.nodes_by_label("Person"), vec![b]);
        graph.delete_node(b);
        assert!(graph.nodes_by_label("Person").is_empty());
    }

    #[test]
    fn test_label_index_add_remove_label() {
        let mut graph = Graph::new();
        let a = graph.create_node("Person");

        graph.add_node_label(a, "Admin".to_string());
        assert_eq!(graph.nodes_by_label("Admin"), vec![a]);
        // Adding a duplicate label must not create duplicate index entries.
        graph.add_node_label(a, "Admin".to_string());
        assert_eq!(graph.nodes_by_label("Admin"), vec![a]);

        graph.remove_node_label(a, "Admin");
        assert!(graph.nodes_by_label("Admin").is_empty());
        // Original label is still indexed.
        assert_eq!(graph.nodes_by_label("Person"), vec![a]);
    }

    #[test]
    fn test_label_index_reused_slot() {
        // After deleting a node, its slot may be reused; the index must not
        // retain the old node under a stale label.
        let mut graph = Graph::new();
        let a = graph.create_node("Person");
        graph.delete_node(a);
        let b = graph.create_node("City"); // likely reuses slot `a`
        assert!(graph.nodes_by_label("Person").is_empty());
        assert_eq!(graph.nodes_by_label("City"), vec![b]);
    }

    #[test]
    fn test_create_node() {
        let mut graph = Graph::new();
        let id = graph.create_node("Person");

        assert_eq!(id, 0);
        assert_eq!(graph.node_count(), 1);

        let node = graph.get_node(id).unwrap();
        assert_eq!(node.primary_label(), "Person");
        assert!(node.has_label("Person"));
    }

    #[test]
    fn test_create_node_with_labels() {
        let mut graph = Graph::new();
        let id = graph.create_node_with_labels(vec![
            "Person".to_string(),
            "Employee".to_string(),
        ]);

        assert_eq!(graph.node_count(), 1);
        let node = graph.get_node(id).unwrap();
        assert!(node.has_label("Person"));
        assert!(node.has_label("Employee"));
        assert_eq!(node.primary_label(), "Person");
        assert_eq!(node.labels.len(), 2);
    }

    #[test]
    fn test_node_add_remove_label() {
        let mut graph = Graph::new();
        let id = graph.create_node("Person");

        {
            let node = graph.get_node_mut(id).unwrap();
            node.add_label("Employee");
            assert!(node.has_label("Employee"));
            assert!(node.has_label("Person"));

            node.remove_label("Person");
            assert!(!node.has_label("Person"));
            assert!(node.has_label("Employee"));
        }
    }

    #[test]
    fn test_create_edge() {
        let mut graph = Graph::new();
        let alice = graph.create_node("Person");
        let bob = graph.create_node("Person");

        let edge_id = graph.create_edge(alice, bob, "KNOWS").unwrap();

        assert_eq!(graph.edge_count(), 1);

        let edge = graph.get_edge(edge_id).unwrap();
        assert_eq!(edge.label, "KNOWS");
        assert_eq!(edge.from, alice);
        assert_eq!(edge.to, bob);
    }

    #[test]
    fn test_node_properties() {
        let mut graph = Graph::new();
        let id = graph.create_node("Person");

        let node = graph.get_node_mut(id).unwrap();
        node.set_property("name", "Alice");
        node.set_property("age", 30);

        let node = graph.get_node(id).unwrap();
        assert_eq!(
            node.get_property("name"),
            Some(&PropertyValue::String("Alice".to_string()))
        );
        assert_eq!(node.get_property("age"), Some(&PropertyValue::Int(30)));
    }

    #[test]
    fn test_get_neighbors() {
        let mut graph = Graph::new();
        let alice = graph.create_node("Person");
        let bob = graph.create_node("Person");
        let charlie = graph.create_node("Person");

        graph.create_edge(alice, bob, "KNOWS").unwrap();
        graph.create_edge(alice, charlie, "KNOWS").unwrap();

        let neighbors = graph.get_neighbors(alice);
        assert_eq!(neighbors.len(), 2);
    }

    #[test]
    fn test_delete_node() {
        let mut graph = Graph::new();
        let alice = graph.create_node("Person");
        let bob = graph.create_node("Person");

        graph.create_edge(alice, bob, "KNOWS").unwrap();
        assert_eq!(graph.edge_count(), 1);

        graph.delete_node(alice);
        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.edge_count(), 0);
    }
}
