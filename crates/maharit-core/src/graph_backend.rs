//! グラフ操作の抽象インターフェース。
//!
//! [`Graph`] と [`ConcurrentGraph`] の両方が実装するトレイトを定義する。
//! エグゼキュータや制約マネージャーはこのトレイトを介してグラフを操作するため、
//! どちらの実装でも透過的に動作する。

use std::sync::Arc;

use crate::concurrent_graph::ConcurrentGraph;
use crate::graph::{Edge, EdgeId, Graph, Node, NodeId};
use crate::property::PropertyValue;
use crate::GraphError;

/// グラフ操作の抽象インターフェース。
///
/// `Graph`（単一スレッド向け密配列グラフ）と `ConcurrentGraph`（DashMap ベース並行グラフ）
/// の両方が実装する。メソッドは所有値を返すため、ライフタイムの問題なく
/// ジェネリックコードから利用できる。
///
/// # 読み取りメソッド
///
/// ノード・エッジを **所有値** として返す（`Arc<HashMap<...>>` は参照カウントのみ
/// インクリメントするため安価なクローン）。
///
/// # 書き込みメソッド
///
/// `&mut self` を要求する。`Graph` では排他アクセスを、`ConcurrentGraph` では
/// DashMap の内部ロックによる同期を使用する。
pub trait GraphBackend: Send + Sync {
    // ── 読み取り操作 ─────────────────────────────────────────────────────────

    /// ノードを所有値として取得する（存在しない場合は `None`）。
    fn get_node(&self, id: NodeId) -> Option<Node>;

    /// 指定ノードがパターンの全ラベルを持つかを、ノード全体をクローンせずに判定する。
    /// ラベルが空スライスの場合は「ノードが存在すれば true」。
    ///
    /// MATCH のスキャン・フィルタのホットパスで `get_node`（Node 全体を複製）を
    /// 避けるための狭い読み取り API。
    fn node_has_all_labels(&self, id: NodeId, labels: &[String]) -> bool;

    /// 指定ノードの単一プロパティ値をクローンして返す（ノード全体はクローンしない）。
    /// ノードまたはプロパティが存在しない場合は `None`。
    fn get_node_property(&self, id: NodeId, key: &str) -> Option<PropertyValue>;

    /// 指定ラベルを持つノード ID を返す（ラベル索引を参照、全走査なし）。
    /// `MATCH (n:Label)` のスキャン起点を該当ラベルのノードだけに絞るために使う。
    fn nodes_by_label(&self, label: &str) -> Vec<NodeId>;

    /// エッジを所有値として取得する（存在しない場合は `None`）。
    fn get_edge(&self, id: EdgeId) -> Option<Edge>;

    /// 全ノードの ID を収集して返す。
    fn node_ids(&self) -> Vec<NodeId>;

    /// 全ノードを所有値として収集して返す。
    fn all_nodes(&self) -> Vec<Node>;

    /// 全エッジの ID を収集して返す。
    fn edge_ids(&self) -> Vec<EdgeId>;

    /// 全エッジを所有値として収集して返す。
    fn all_edges(&self) -> Vec<Edge>;

    /// 指定ノードの出向きエッジを所有値として返す。
    fn outgoing_edges(&self, node_id: NodeId) -> Vec<Edge>;

    /// 指定ノードの入向きエッジを所有値として返す。
    fn incoming_edges(&self, node_id: NodeId) -> Vec<Edge>;

    /// 指定ノードに隣接エッジが 1 本以上あるか。
    fn has_incident_edges(&self, node_id: NodeId) -> bool;

    /// ライブなノード数を返す。
    fn node_count(&self) -> usize;

    /// ライブなエッジ数を返す。
    fn edge_count(&self) -> usize;

    /// 指定 ID のノードが存在するか。
    fn contains_node(&self, id: NodeId) -> bool;

    // ── 書き込み操作 ─────────────────────────────────────────────────────────

    /// 複数ラベルを持つノードを作成して ID を返す。
    fn create_node_with_labels(&mut self, labels: Vec<String>) -> NodeId;

    /// エッジを作成して ID を返す。
    fn create_edge(
        &mut self,
        from: NodeId,
        to: NodeId,
        label: String,
    ) -> Result<EdgeId, GraphError>;

    /// ノードを削除して返す（存在しない場合は `None`）。
    fn delete_node(&mut self, id: NodeId) -> Option<Node>;

    /// エッジを削除して返す（存在しない場合は `None`）。
    fn delete_edge(&mut self, id: EdgeId) -> Option<Edge>;

    /// ノードにプロパティを設定する。
    fn set_node_property(&mut self, id: NodeId, key: &str, value: PropertyValue);

    /// ノードのプロパティを削除して返す。
    fn remove_node_property(&mut self, id: NodeId, key: &str) -> Option<PropertyValue>;

    /// ノードにラベルを追加する（重複は無視）。
    fn add_node_label(&mut self, id: NodeId, label: String);

    /// ノードのラベルを削除する。
    fn remove_node_label(&mut self, id: NodeId, label: &str);

    /// エッジにプロパティを設定する。
    fn set_edge_property(&mut self, id: EdgeId, key: &str, value: PropertyValue);

    /// エッジのプロパティを削除して返す。
    fn remove_edge_property(&mut self, id: EdgeId, key: &str) -> Option<PropertyValue>;
}

// ── Graph 実装 ─────────────────────────────────────────────────────────────

impl GraphBackend for Graph {
    fn get_node(&self, id: NodeId) -> Option<Node> {
        // 同名の固有メソッドが &Node を返すのでクローンして所有値にする。
        Graph::get_node(self, id).cloned()
    }

    fn node_has_all_labels(&self, id: NodeId, labels: &[String]) -> bool {
        match Graph::get_node(self, id) {
            Some(n) => labels.iter().all(|l| n.has_label(l)),
            None => false,
        }
    }

    fn get_node_property(&self, id: NodeId, key: &str) -> Option<PropertyValue> {
        Graph::get_node(self, id).and_then(|n| n.get_property(key).cloned())
    }

    fn nodes_by_label(&self, label: &str) -> Vec<NodeId> {
        Graph::nodes_by_label(self, label)
    }

    fn get_edge(&self, id: EdgeId) -> Option<Edge> {
        Graph::get_edge(self, id).cloned()
    }

    fn node_ids(&self) -> Vec<NodeId> {
        self.nodes().map(|n| n.id).collect()
    }

    fn all_nodes(&self) -> Vec<Node> {
        self.nodes().cloned().collect()
    }

    fn edge_ids(&self) -> Vec<EdgeId> {
        self.edges().map(|e| e.id).collect()
    }

    fn all_edges(&self) -> Vec<Edge> {
        self.edges().cloned().collect()
    }

    fn outgoing_edges(&self, node_id: NodeId) -> Vec<Edge> {
        Graph::get_outgoing_edges(self, node_id)
            .cloned()
            .collect()
    }

    fn incoming_edges(&self, node_id: NodeId) -> Vec<Edge> {
        Graph::get_incoming_edges(self, node_id)
            .cloned()
            .collect()
    }

    fn has_incident_edges(&self, node_id: NodeId) -> bool {
        Graph::get_outgoing_edges(self, node_id).next().is_some()
            || Graph::get_incoming_edges(self, node_id).next().is_some()
    }

    fn node_count(&self) -> usize {
        Graph::node_count(self)
    }

    fn edge_count(&self) -> usize {
        Graph::edge_count(self)
    }

    fn contains_node(&self, id: NodeId) -> bool {
        Graph::get_node(self, id).is_some()
    }

    fn create_node_with_labels(&mut self, labels: Vec<String>) -> NodeId {
        Graph::create_node_with_labels(self, labels)
    }

    fn create_edge(
        &mut self,
        from: NodeId,
        to: NodeId,
        label: String,
    ) -> Result<EdgeId, GraphError> {
        Graph::create_edge(self, from, to, label)
    }

    fn delete_node(&mut self, id: NodeId) -> Option<Node> {
        Graph::delete_node(self, id)
    }

    fn delete_edge(&mut self, id: EdgeId) -> Option<Edge> {
        Graph::delete_edge(self, id)
    }

    fn set_node_property(&mut self, id: NodeId, key: &str, value: PropertyValue) {
        if let Some(node) = Graph::get_node_mut(self, id) {
            node.set_property(key, value);
        }
    }

    fn remove_node_property(&mut self, id: NodeId, key: &str) -> Option<PropertyValue> {
        Graph::get_node_mut(self, id)?.remove_property(key)
    }

    fn add_node_label(&mut self, id: NodeId, label: String) {
        // ラベル索引も更新する Graph 側メソッド経由で行う。
        Graph::add_node_label(self, id, label);
    }

    fn remove_node_label(&mut self, id: NodeId, label: &str) {
        Graph::remove_node_label(self, id, label);
    }

    fn set_edge_property(&mut self, id: EdgeId, key: &str, value: PropertyValue) {
        if let Some(edge) = Graph::get_edge_mut(self, id) {
            edge.set_property(key, value);
        }
    }

    fn remove_edge_property(&mut self, id: EdgeId, key: &str) -> Option<PropertyValue> {
        Graph::get_edge_mut(self, id)?.remove_property(key)
    }
}

// ── ConcurrentGraph 実装 ─────────────────────────────────────────────────

impl GraphBackend for ConcurrentGraph {
    fn get_node(&self, id: NodeId) -> Option<Node> {
        self.with_node(id, |n| n.clone())
    }

    fn node_has_all_labels(&self, id: NodeId, labels: &[String]) -> bool {
        self.with_node(id, |n| labels.iter().all(|l| n.has_label(l)))
            .unwrap_or(false)
    }

    fn get_node_property(&self, id: NodeId, key: &str) -> Option<PropertyValue> {
        self.with_node(id, |n| n.get_property(key).cloned()).flatten()
    }

    fn nodes_by_label(&self, label: &str) -> Vec<NodeId> {
        ConcurrentGraph::nodes_by_label(self, label)
    }

    fn get_edge(&self, id: EdgeId) -> Option<Edge> {
        self.with_edge(id, |e| e.clone())
    }

    fn node_ids(&self) -> Vec<NodeId> {
        self.nodes().map(|r| *r.key()).collect()
    }

    fn all_nodes(&self) -> Vec<Node> {
        self.nodes().map(|r| r.value().clone()).collect()
    }

    fn edge_ids(&self) -> Vec<EdgeId> {
        self.edges().map(|r| *r.key()).collect()
    }

    fn all_edges(&self) -> Vec<Edge> {
        self.edges().map(|r| r.value().clone()).collect()
    }

    fn outgoing_edges(&self, node_id: NodeId) -> Vec<Edge> {
        self.get_outgoing_edges(node_id)
            .into_iter()
            .filter_map(|eid| self.with_edge(eid, |e| e.clone()))
            .collect()
    }

    fn incoming_edges(&self, node_id: NodeId) -> Vec<Edge> {
        self.get_incoming_edges(node_id)
            .into_iter()
            .filter_map(|eid| self.with_edge(eid, |e| e.clone()))
            .collect()
    }

    fn has_incident_edges(&self, node_id: NodeId) -> bool {
        !self.get_outgoing_edges(node_id).is_empty()
            || !self.get_incoming_edges(node_id).is_empty()
    }

    fn node_count(&self) -> usize {
        ConcurrentGraph::node_count(self)
    }

    fn edge_count(&self) -> usize {
        ConcurrentGraph::edge_count(self)
    }

    fn contains_node(&self, id: NodeId) -> bool {
        ConcurrentGraph::contains_node(self, id)
    }

    fn create_node_with_labels(&mut self, labels: Vec<String>) -> NodeId {
        ConcurrentGraph::create_node_with_labels(self, labels)
    }

    fn create_edge(
        &mut self,
        from: NodeId,
        to: NodeId,
        label: String,
    ) -> Result<EdgeId, GraphError> {
        ConcurrentGraph::create_edge(self, from, to, label)
    }

    fn delete_node(&mut self, id: NodeId) -> Option<Node> {
        ConcurrentGraph::delete_node(self, id)
    }

    fn delete_edge(&mut self, id: EdgeId) -> Option<Edge> {
        ConcurrentGraph::delete_edge(self, id)
    }

    fn set_node_property(&mut self, id: NodeId, key: &str, value: PropertyValue) {
        ConcurrentGraph::set_node_property(self, id, key, value);
    }

    fn remove_node_property(&mut self, id: NodeId, key: &str) -> Option<PropertyValue> {
        self.with_node_mut(id, |n| Arc::make_mut(&mut n.properties).remove(key))
            .flatten()
    }

    fn add_node_label(&mut self, id: NodeId, label: String) {
        ConcurrentGraph::add_node_label(self, id, label);
    }

    fn remove_node_label(&mut self, id: NodeId, label: &str) {
        ConcurrentGraph::remove_node_label(self, id, label);
    }

    fn set_edge_property(&mut self, id: EdgeId, key: &str, value: PropertyValue) {
        ConcurrentGraph::set_edge_property(self, id, key, value);
    }

    fn remove_edge_property(&mut self, id: EdgeId, key: &str) -> Option<PropertyValue> {
        self.with_edge_mut(id, |e| Arc::make_mut(&mut e.properties).remove(key))
            .flatten()
    }
}
