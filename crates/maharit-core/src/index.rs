use std::collections::{HashMap, HashSet};

use crate::{EdgeId, NodeId};

/// ラベルインデックス
#[derive(Debug, Default)]
pub struct LabelIndex {
    /// ノードラベル -> ノードID集合
    node_labels: HashMap<String, HashSet<NodeId>>,
    /// ノードID -> ラベル集合（逆引き用、複数ラベル対応）
    /// HashSet により重複チェックが O(N) → O(1) になる
    node_to_labels: HashMap<NodeId, HashSet<String>>,
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

    /// ノードをインデックスに追加（単一ラベル版、後方互換のため残す）
    pub fn add_node(&mut self, node_id: NodeId, label: &str) {
        if label.is_empty() {
            return;
        }
        self.add_node_labels(node_id, &[label]);
    }

    /// ノードを複数ラベルでインデックスに追加する
    pub fn add_node_labels(&mut self, node_id: NodeId, labels: &[&str]) {
        for &label in labels {
            if label.is_empty() {
                continue;
            }
            // Forward index: label → node set
            // entry() needs an owned key; one allocation per new label
            self.node_labels
                .entry(label.to_string())
                .or_default()
                .insert(node_id);
            // Reverse index: node → label set (HashSet deduplicates in O(1))
            self.node_to_labels
                .entry(node_id)
                .or_default()
                .insert(label.to_string());
        }
    }

    /// ノードをインデックスから削除（全ラベルを削除）
    pub fn remove_node(&mut self, node_id: NodeId) {
        if let Some(labels) = self.node_to_labels.remove(&node_id) {
            for label in labels {
                if let Some(nodes) = self.node_labels.get_mut(&label) {
                    nodes.remove(&node_id);
                    if nodes.is_empty() {
                        self.node_labels.remove(&label);
                    }
                }
            }
        }
    }

    /// ノードから特定のラベルをインデックスから削除する
    pub fn remove_node_label(&mut self, node_id: NodeId, label: &str) {
        if let Some(labels) = self.node_to_labels.get_mut(&node_id) {
            labels.remove(label);
            if labels.is_empty() {
                self.node_to_labels.remove(&node_id);
            }
        }
        if let Some(nodes) = self.node_labels.get_mut(label) {
            nodes.remove(&node_id);
            if nodes.is_empty() {
                self.node_labels.remove(label);
            }
        }
    }

    /// ノードのラベルを変更し、インデックスを更新する（後方互換のため残す）
    ///
    /// 旧ラベルのインデックスから削除し、新ラベルのインデックスへ追加する。
    ///
    /// # Arguments
    ///
    /// * `node_id` - 更新対象のノードID
    /// * `new_label` - 新しいラベル
    ///
    /// # Returns
    ///
    /// 変更前の最初のラベル（存在していた場合は `Some(old_label)`、なければ `None`）
    pub fn update_node_label(&mut self, node_id: NodeId, new_label: &str) -> Option<String> {
        // 旧ラベルをインデックスから削除
        let old_first = if let Some(labels) = self.node_to_labels.remove(&node_id) {
            let first = labels.iter().next().cloned();
            for label in labels {
                if let Some(nodes) = self.node_labels.get_mut(&label) {
                    nodes.remove(&node_id);
                    if nodes.is_empty() {
                        self.node_labels.remove(&label);
                    }
                }
            }
            first
        } else {
            None
        };

        // 新ラベルをインデックスに追加（空ラベルは無視）
        if !new_label.is_empty() {
            self.node_labels
                .entry(new_label.to_string())
                .or_default()
                .insert(node_id);
            let mut label_set = HashSet::new();
            label_set.insert(new_label.to_string());
            self.node_to_labels.insert(node_id, label_set);
        }

        old_first
    }

    /// ノードが持つラベルリストを取得する
    pub fn get_node_labels(&self, node_id: NodeId) -> Vec<&str> {
        self.node_to_labels
            .get(&node_id)
            .map(|labels| labels.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// ラベルでノードをイテレートする（Vec アロケーションなし）
    pub fn get_nodes_by_label(&self, label: &str) -> impl Iterator<Item = NodeId> + '_ {
        self.node_labels
            .get(label)
            .into_iter()
            .flat_map(|nodes| nodes.iter().copied())
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
        if let Some(label) = self.edge_to_label.remove(&edge_id)
            && let Some(edges) = self.edge_labels.get_mut(&label)
        {
            edges.remove(&edge_id);
            if edges.is_empty() {
                self.edge_labels.remove(&label);
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
        self.edge_labels
            .get(edge_type)
            .map(|e| e.len())
            .unwrap_or(0)
    }

    /// 全てのエッジラベルを取得
    pub fn all_edge_labels(&self) -> Vec<&str> {
        self.edge_labels.keys().map(|s| s.as_str()).collect()
    }

    // ========== 統計 ==========

    /// インデックス済みノード数
    pub fn indexed_node_count(&self) -> usize {
        self.node_to_labels.len()
    }

    /// インデックス済みエッジ数
    pub fn indexed_edge_count(&self) -> usize {
        self.edge_to_label.len()
    }

    /// インデックスをクリア
    pub fn clear(&mut self) {
        self.node_labels.clear();
        self.node_to_labels.clear();
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

        let persons: Vec<_> = index.get_nodes_by_label("Person").collect();
        assert_eq!(persons.len(), 2);
        assert!(persons.contains(&0));
        assert!(persons.contains(&1));

        let companies: Vec<_> = index.get_nodes_by_label("Company").collect();
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
        assert!(index.get_nodes_by_label("Person").next().is_none());
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

        assert!(index.get_nodes_by_label("NonExistent").next().is_none());
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

    #[test]
    fn test_update_node_label_new_label_queryable() {
        // ラベル変更後、新ラベルで検索できること
        let mut index = LabelIndex::new();
        index.add_node(0, "Person");
        index.add_node(1, "Person");

        // ノード0のラベルを Person -> Employee に変更
        let old_label = index.update_node_label(0, "Employee");
        assert_eq!(old_label, Some("Person".to_string()));

        // 新ラベル "Employee" で検索できる
        let employees: Vec<_> = index.get_nodes_by_label("Employee").collect();
        assert_eq!(employees.len(), 1);
        assert!(employees.contains(&0));
    }

    #[test]
    fn test_update_node_label_old_label_not_queryable() {
        // ラベル変更後、旧ラベルでは検索できないこと
        let mut index = LabelIndex::new();
        index.add_node(0, "Person");
        index.add_node(1, "Person");

        // ノード0のラベルを Person -> Employee に変更
        index.update_node_label(0, "Employee");

        // 旧ラベル "Person" でノード0は見つからない
        let persons: Vec<_> = index.get_nodes_by_label("Person").collect();
        assert_eq!(persons.len(), 1);
        assert!(!persons.contains(&0), "旧ラベルで変更したノードは見つからないべき");
        assert!(persons.contains(&1), "旧ラベルを持つ他のノードは見つかるべき");
    }

    #[test]
    fn test_update_node_label_removes_empty_old_label() {
        // 旧ラベルのノードが0になった場合、ラベルエントリが削除される
        let mut index = LabelIndex::new();
        index.add_node(0, "Temp");

        index.update_node_label(0, "Permanent");

        // 旧ラベル "Temp" のノードは空になる
        assert!(index.get_nodes_by_label("Temp").next().is_none());
        // 新ラベル "Permanent" は正常に機能する
        assert_eq!(index.get_nodes_by_label("Permanent").count(), 1);
    }

    #[test]
    fn test_update_node_label_returns_none_for_unregistered_node() {
        // 未登録ノードの更新は None を返す
        let mut index = LabelIndex::new();

        let old_label = index.update_node_label(999, "NewLabel");
        assert_eq!(old_label, None);

        // 新ラベルは登録される
        let nodes: Vec<_> = index.get_nodes_by_label("NewLabel").collect();
        assert_eq!(nodes.len(), 1);
        assert!(nodes.contains(&999));
    }

    #[test]
    fn test_add_node_labels_multiple() {
        let mut index = LabelIndex::new();

        // ノード0 に Person と Employee の2ラベルを付与
        index.add_node_labels(0, &["Person", "Employee"]);

        let persons: Vec<_> = index.get_nodes_by_label("Person").collect();
        assert_eq!(persons.len(), 1);
        assert!(persons.contains(&0));

        let employees: Vec<_> = index.get_nodes_by_label("Employee").collect();
        assert_eq!(employees.len(), 1);
        assert!(employees.contains(&0));

        // 逆引きで2ラベルを確認
        let labels = index.get_node_labels(0);
        assert_eq!(labels.len(), 2);
        assert!(labels.contains(&"Person"));
        assert!(labels.contains(&"Employee"));
    }

    #[test]
    fn test_add_node_labels_incremental() {
        let mut index = LabelIndex::new();

        index.add_node(0, "Person");
        index.add_node_labels(0, &["Employee"]);

        // 両方のラベルで検索できる
        assert!(index.get_nodes_by_label("Person").any(|id| id == 0));
        assert!(index.get_nodes_by_label("Employee").any(|id| id == 0));

        // 1ノードとしてカウント
        assert_eq!(index.indexed_node_count(), 1);
    }

    #[test]
    fn test_remove_node_label_specific() {
        let mut index = LabelIndex::new();

        index.add_node_labels(0, &["Person", "Employee"]);

        // Employeeラベルだけ削除
        index.remove_node_label(0, "Employee");

        // Personラベルはまだある
        assert!(index.get_nodes_by_label("Person").any(|id| id == 0));
        assert!(!index.get_nodes_by_label("Employee").any(|id| id == 0));

        // ノード自体はまだ登録されている
        assert_eq!(index.indexed_node_count(), 1);
    }

    #[test]
    fn test_remove_node_label_last_removes_node() {
        let mut index = LabelIndex::new();

        index.add_node(0, "Person");
        index.remove_node_label(0, "Person");

        // 全ラベルが消えたのでノードは登録解除される
        assert_eq!(index.indexed_node_count(), 0);
        assert!(index.get_nodes_by_label("Person").next().is_none());
    }
}
