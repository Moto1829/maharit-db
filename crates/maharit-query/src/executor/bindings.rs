//! パターン照合中の変数束縛（`BindingValue` と、変数名からの対応表 `Bindings`）。

use super::*;

/// バインディング値
#[derive(Debug, Clone, PartialEq)]
pub enum BindingValue {
    /// 単一ノード
    Node(NodeId),
    /// 単一エッジ
    Edge(u64),
    /// パス（可変長パス用）
    Path { nodes: Vec<NodeId>, edges: Vec<u64> },
    /// スカラー値（WITH句で渡される中間値）
    Scalar(Value),
}

impl BindingValue {
    /// ノードIDを取得（Nodeの場合のみ）
    pub fn as_node(&self) -> Option<NodeId> {
        match self {
            BindingValue::Node(id) => Some(*id),
            _ => None,
        }
    }

    /// エッジIDを取得（Edgeの場合のみ）
    pub fn as_edge(&self) -> Option<u64> {
        match self {
            BindingValue::Edge(id) => Some(*id),
            _ => None,
        }
    }
}

/// 変数バインディング
// 変数名は `Arc<str>` をキーにする。多段パターン/JOIN で bindings を複製する際、
// キー文字列を再確保せず参照カウントのインクリメントで済む（clone が浅くなる）。
// 変数名インターナーは持たず、insert 時に `Arc::from(var)` で確保する。
pub(super) type Bindings = HashMap<Arc<str>, BindingValue>;
