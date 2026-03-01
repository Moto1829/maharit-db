use std::collections::HashMap;

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
    pub properties: HashMap<String, PropertyValue>,
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
        self.properties.insert(key.into(), value.into());
    }

    /// プロパティを取得
    pub fn get_property(&self, key: &str) -> Option<&PropertyValue> {
        self.properties.get(key)
    }

    /// プロパティを削除
    pub fn remove_property(&mut self, key: &str) -> Option<PropertyValue> {
        self.properties.remove(key)
    }
}

/// エッジ（辺）を表す構造体
#[derive(Debug, Clone)]
pub struct Edge {
    pub id: EdgeId,
    pub label: String,
    pub from: NodeId,
    pub to: NodeId,
    pub properties: HashMap<String, PropertyValue>,
}

impl Edge {
    /// プロパティを設定
    pub fn set_property(&mut self, key: impl Into<String>, value: impl Into<PropertyValue>) {
        self.properties.insert(key.into(), value.into());
    }

    /// プロパティを取得
    pub fn get_property(&self, key: &str) -> Option<&PropertyValue> {
        self.properties.get(key)
    }

    /// プロパティを削除
    pub fn remove_property(&mut self, key: &str) -> Option<PropertyValue> {
        self.properties.remove(key)
    }
}

/// インメモリグラフデータベース
#[derive(Debug, Default)]
pub struct Graph {
    nodes: HashMap<NodeId, Node>,
    edges: HashMap<EdgeId, Edge>,
    /// ノードから出るエッジのインデックス (from_node_id -> [edge_id])
    outgoing_edges: HashMap<NodeId, Vec<EdgeId>>,
    /// ノードに入るエッジのインデックス (to_node_id -> [edge_id])
    incoming_edges: HashMap<NodeId, Vec<EdgeId>>,
    next_node_id: NodeId,
    next_edge_id: EdgeId,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
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
        let id = self.next_node_id;
        self.next_node_id += 1;

        let node = Node {
            id,
            labels,
            properties: HashMap::new(),
        };

        self.nodes.insert(id, node);
        self.outgoing_edges.insert(id, Vec::new());
        self.incoming_edges.insert(id, Vec::new());

        id
    }

    /// 指定したIDでノードを作成する（バックアップ/リストア専用、単一ラベル版）
    ///
    /// 指定した `id` が既に使用されている場合は既存ノードの ID を返す。
    /// `next_node_id` は必要に応じて更新される。
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
        if self.nodes.contains_key(&id) {
            return id;
        }

        let node = Node {
            id,
            labels,
            properties: HashMap::new(),
        };

        self.nodes.insert(id, node);
        self.outgoing_edges.insert(id, Vec::new());
        self.incoming_edges.insert(id, Vec::new());

        // Keep next_node_id ahead of all assigned IDs.
        if id >= self.next_node_id {
            self.next_node_id = id + 1;
        }

        id
    }

    /// ノードを取得
    pub fn get_node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    /// ノードを可変参照で取得
    pub fn get_node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(&id)
    }

    /// ノードを削除
    pub fn delete_node(&mut self, id: NodeId) -> Option<Node> {
        // 関連するエッジを削除
        if let Some(outgoing) = self.outgoing_edges.remove(&id) {
            for edge_id in outgoing {
                if let Some(edge) = self.edges.remove(&edge_id) {
                    if let Some(incoming) = self.incoming_edges.get_mut(&edge.to) {
                        incoming.retain(|&e| e != edge_id);
                    }
                }
            }
        }

        if let Some(incoming) = self.incoming_edges.remove(&id) {
            for edge_id in incoming {
                if let Some(edge) = self.edges.remove(&edge_id) {
                    if let Some(outgoing) = self.outgoing_edges.get_mut(&edge.from) {
                        outgoing.retain(|&e| e != edge_id);
                    }
                }
            }
        }

        self.nodes.remove(&id)
    }

    /// 新しいエッジを作成して追加
    pub fn create_edge(
        &mut self,
        from: NodeId,
        to: NodeId,
        label: impl Into<String>,
    ) -> Result<EdgeId, GraphError> {
        if !self.nodes.contains_key(&from) {
            return Err(GraphError::NodeNotFound(from));
        }
        if !self.nodes.contains_key(&to) {
            return Err(GraphError::NodeNotFound(to));
        }

        let id = self.next_edge_id;
        self.next_edge_id += 1;

        let edge = Edge {
            id,
            label: label.into(),
            from,
            to,
            properties: HashMap::new(),
        };

        self.edges.insert(id, edge);
        self.outgoing_edges.get_mut(&from).unwrap().push(id);
        self.incoming_edges.get_mut(&to).unwrap().push(id);

        Ok(id)
    }

    /// エッジを取得
    pub fn get_edge(&self, id: EdgeId) -> Option<&Edge> {
        self.edges.get(&id)
    }

    /// エッジを可変参照で取得
    pub fn get_edge_mut(&mut self, id: EdgeId) -> Option<&mut Edge> {
        self.edges.get_mut(&id)
    }

    /// エッジを削除
    pub fn delete_edge(&mut self, id: EdgeId) -> Option<Edge> {
        if let Some(edge) = self.edges.remove(&id) {
            if let Some(outgoing) = self.outgoing_edges.get_mut(&edge.from) {
                outgoing.retain(|&e| e != id);
            }
            if let Some(incoming) = self.incoming_edges.get_mut(&edge.to) {
                incoming.retain(|&e| e != id);
            }
            Some(edge)
        } else {
            None
        }
    }

    /// ノードから出るエッジを取得
    pub fn get_outgoing_edges(&self, node_id: NodeId) -> Vec<&Edge> {
        self.outgoing_edges
            .get(&node_id)
            .map(|edge_ids| {
                edge_ids
                    .iter()
                    .filter_map(|id| self.edges.get(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// ノードに入るエッジを取得
    pub fn get_incoming_edges(&self, node_id: NodeId) -> Vec<&Edge> {
        self.incoming_edges
            .get(&node_id)
            .map(|edge_ids| {
                edge_ids
                    .iter()
                    .filter_map(|id| self.edges.get(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 隣接ノードを取得（出るエッジの先）
    pub fn get_neighbors(&self, node_id: NodeId) -> Vec<&Node> {
        self.get_outgoing_edges(node_id)
            .iter()
            .filter_map(|edge| self.nodes.get(&edge.to))
            .collect()
    }

    /// 全ノード数
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// 全エッジ数
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// 全ノードをイテレート
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    /// 全エッジをイテレート
    pub fn edges(&self) -> impl Iterator<Item = &Edge> {
        self.edges.values()
    }

    /// 指定したノードからトラバーサルを開始
    pub fn traverse(&self, start: NodeId) -> Traversal<'_> {
        Traversal::new(self, start)
    }

    /// ラベルでノードを検索（指定ラベルを持つノードを全て返す）
    pub fn find_nodes_by_label(&self, label: &str) -> Vec<&Node> {
        self.nodes
            .values()
            .filter(|n| n.has_label(label))
            .collect()
    }

    /// ラベル（タイプ）でエッジを検索
    pub fn find_edges_by_type(&self, edge_type: &str) -> Vec<&Edge> {
        self.edges
            .values()
            .filter(|e| e.label == edge_type)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
