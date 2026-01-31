use std::collections::{HashMap, HashSet};

use crate::{EdgeId, NodeId};

/// ラベルインデックス
#[derive(Debug, Default)]
pub struct LabelIndex {
    /// ノードラベル -> ノードID集合
    node_labels: HashMap<String, HashSet<NodeId>>,
    /// ノードID -> ラベル（逆引き用）
    node_to_label: HashMap<NodeId, String>,
    /// エッジラベル -> エッジID集合
    edge_labels: HashMap<String, HashSet<EdgeId>>,
    /// エッジID -> ラベル（逆引き用）
    edge_to_label: HashMap<EdgeId, String>,
}

impl LabelIndex {
    pub fn new() -> Self {
        Self::default()
    }

    // ========== ノードインデックス操作 ==========

    /// ノードをインデックスに追加
    pub fn add_node(&mut self, node_id: NodeId, label: &str) {
        if label.is_empty() {
            return;
        }

        self.node_labels
            .entry(label.to_string())
            .or_default()
            .insert(node_id);
        self.node_to_label.insert(node_id, label.to_string());
    }

    /// ノードをインデックスから削除
    pub fn remove_node(&mut self, node_id: NodeId) {
        if let Some(label) = self.node_to_label.remove(&node_id) {
            if let Some(nodes) = self.node_labels.get_mut(&label) {
                nodes.remove(&node_id);
                if nodes.is_empty() {
                    self.node_labels.remove(&label);
                }
            }
        }
    }

    /// ラベルでノードを検索
    pub fn get_nodes_by_label(&self, label: &str) -> Vec<NodeId> {
        self.node_labels
            .get(label)
            .map(|nodes| nodes.iter().copied().collect())
            .unwrap_or_default()
    }

    /// ラベルを持つノード数をカウント
    pub fn count_nodes_by_label(&self, label: &str) -> usize {
        self.node_labels.get(label).map(|n| n.len()).unwrap_or(0)
    }

    /// 全てのノードラベルを取得
    pub fn all_node_labels(&self) -> Vec<&str> {
        self.node_labels.keys().map(|s| s.as_str()).collect()
    }

    // ========== エッジインデックス操作 ==========

    /// エッジをインデックスに追加
    pub fn add_edge(&mut self, edge_id: EdgeId, label: &str) {
        if label.is_empty() {
            return;
        }

        self.edge_labels
            .entry(label.to_string())
            .or_default()
            .insert(edge_id);
        self.edge_to_label.insert(edge_id, label.to_string());
    }

    /// エッジをインデックスから削除
    pub fn remove_edge(&mut self, edge_id: EdgeId) {
        if let Some(label) = self.edge_to_label.remove(&edge_id) {
            if let Some(edges) = self.edge_labels.get_mut(&label) {
                edges.remove(&edge_id);
                if edges.is_empty() {
                    self.edge_labels.remove(&label);
                }
            }
        }
    }

    /// ラベル（タイプ）でエッジを検索
    pub fn get_edges_by_type(&self, edge_type: &str) -> Vec<EdgeId> {
        self.edge_labels
            .get(edge_type)
            .map(|edges| edges.iter().copied().collect())
            .unwrap_or_default()
    }

    /// ラベルを持つエッジ数をカウント
    pub fn count_edges_by_type(&self, edge_type: &str) -> usize {
        self.edge_labels.get(edge_type).map(|e| e.len()).unwrap_or(0)
    }

    /// 全てのエッジラベルを取得
    pub fn all_edge_labels(&self) -> Vec<&str> {
        self.edge_labels.keys().map(|s| s.as_str()).collect()
    }

    // ========== 統計 ==========

    /// インデックス済みノード数
    pub fn indexed_node_count(&self) -> usize {
        self.node_to_label.len()
    }

    /// インデックス済みエッジ数
    pub fn indexed_edge_count(&self) -> usize {
        self.edge_to_label.len()
    }

    /// インデックスをクリア
    pub fn clear(&mut self) {
        self.node_labels.clear();
        self.node_to_label.clear();
        self.edge_labels.clear();
        self.edge_to_label.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_get_nodes_by_label() {
        let mut index = LabelIndex::new();

        index.add_node(0, "Person");
        index.add_node(1, "Person");
        index.add_node(2, "Company");

        let persons = index.get_nodes_by_label("Person");
        assert_eq!(persons.len(), 2);
        assert!(persons.contains(&0));
        assert!(persons.contains(&1));

        let companies = index.get_nodes_by_label("Company");
        assert_eq!(companies.len(), 1);
        assert!(companies.contains(&2));
    }

    #[test]
    fn test_remove_node() {
        let mut index = LabelIndex::new();

        index.add_node(0, "Person");
        index.add_node(1, "Person");

        assert_eq!(index.count_nodes_by_label("Person"), 2);

        index.remove_node(0);
        assert_eq!(index.count_nodes_by_label("Person"), 1);

        index.remove_node(1);
        assert_eq!(index.count_nodes_by_label("Person"), 0);

        // ラベル自体も削除されている
        assert!(index.get_nodes_by_label("Person").is_empty());
    }

    #[test]
    fn test_add_and_get_edges_by_type() {
        let mut index = LabelIndex::new();

        index.add_edge(0, "KNOWS");
        index.add_edge(1, "KNOWS");
        index.add_edge(2, "WORKS_AT");

        let knows = index.get_edges_by_type("KNOWS");
        assert_eq!(knows.len(), 2);
        assert!(knows.contains(&0));
        assert!(knows.contains(&1));

        let works_at = index.get_edges_by_type("WORKS_AT");
        assert_eq!(works_at.len(), 1);
        assert!(works_at.contains(&2));
    }

    #[test]
    fn test_remove_edge() {
        let mut index = LabelIndex::new();

        index.add_edge(0, "KNOWS");
        index.add_edge(1, "KNOWS");

        assert_eq!(index.count_edges_by_type("KNOWS"), 2);

        index.remove_edge(0);
        assert_eq!(index.count_edges_by_type("KNOWS"), 1);
    }

    #[test]
    fn test_all_labels() {
        let mut index = LabelIndex::new();

        index.add_node(0, "Person");
        index.add_node(1, "Company");
        index.add_edge(0, "KNOWS");
        index.add_edge(1, "WORKS_AT");

        let node_labels = index.all_node_labels();
        assert_eq!(node_labels.len(), 2);
        assert!(node_labels.contains(&"Person"));
        assert!(node_labels.contains(&"Company"));

        let edge_labels = index.all_edge_labels();
        assert_eq!(edge_labels.len(), 2);
        assert!(edge_labels.contains(&"KNOWS"));
        assert!(edge_labels.contains(&"WORKS_AT"));
    }

    #[test]
    fn test_empty_label_ignored() {
        let mut index = LabelIndex::new();

        index.add_node(0, "");
        index.add_edge(0, "");

        assert_eq!(index.indexed_node_count(), 0);
        assert_eq!(index.indexed_edge_count(), 0);
    }

    #[test]
    fn test_nonexistent_label() {
        let index = LabelIndex::new();

        assert!(index.get_nodes_by_label("NonExistent").is_empty());
        assert_eq!(index.count_nodes_by_label("NonExistent"), 0);
    }

    #[test]
    fn test_clear() {
        let mut index = LabelIndex::new();

        index.add_node(0, "Person");
        index.add_edge(0, "KNOWS");

        assert_eq!(index.indexed_node_count(), 1);
        assert_eq!(index.indexed_edge_count(), 1);

        index.clear();

        assert_eq!(index.indexed_node_count(), 0);
        assert_eq!(index.indexed_edge_count(), 0);
    }
}
