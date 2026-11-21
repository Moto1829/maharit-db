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

/// 変数バインディング（1 行ぶんの「変数名 → 値」）。
///
/// 1 行の変数は数個なので、ハッシュ表ではなく小さな配列に入れて線形に探す（task188）。
/// 行を複製するとき（1 ホップ・1 行ごと）にハッシュ表の確保と、探すたびの
/// 文字列のハッシュ計算が要らなくなる。変数名は `Arc<str>` なので複製は参照カウントだけ。
///
/// `HashMap` と同じ名前のメソッドを持つ（呼び出し側は前と同じに書ける）。
#[derive(Default)]
pub(super) struct Bindings {
    entries: Vec<(Arc<str>, BindingValue)>,
}

impl Bindings {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn get(&self, key: &str) -> Option<&BindingValue> {
        self.entries
            .iter()
            .find(|(k, _)| &**k == key)
            .map(|(_, v)| v)
    }

    pub(super) fn get_mut(&mut self, key: &str) -> Option<&mut BindingValue> {
        self.entries
            .iter_mut()
            .find(|(k, _)| &**k == key)
            .map(|(_, v)| v)
    }

    pub(super) fn contains_key(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _)| &**k == key)
    }

    /// 同じ名前があれば値を置き換え、前の値を返す。
    pub(super) fn insert(&mut self, key: Arc<str>, value: BindingValue) -> Option<BindingValue> {
        match self.get_mut(&key) {
            Some(slot) => Some(std::mem::replace(slot, value)),
            None => {
                self.entries.push((key, value));
                None
            }
        }
    }

    pub(super) fn retain(&mut self, mut f: impl FnMut(&Arc<str>, &mut BindingValue) -> bool) {
        self.entries.retain_mut(|(k, v)| f(k, v));
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (&Arc<str>, &BindingValue)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    pub(super) fn values(&self) -> impl Iterator<Item = &BindingValue> {
        self.entries.iter().map(|(_, v)| v)
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// 複製した行には、たいてい直後に変数を 1〜2 個足すので、その分の容量を先に取っておく
/// （足すたびの確保し直しを避ける）。
impl Clone for Bindings {
    fn clone(&self) -> Self {
        let mut entries = Vec::with_capacity(self.entries.len() + 2);
        entries.extend(self.entries.iter().cloned());
        Self { entries }
    }
}

impl Extend<(Arc<str>, BindingValue)> for Bindings {
    fn extend<I: IntoIterator<Item = (Arc<str>, BindingValue)>>(&mut self, iter: I) {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }
}

impl FromIterator<(Arc<str>, BindingValue)> for Bindings {
    fn from_iter<I: IntoIterator<Item = (Arc<str>, BindingValue)>>(iter: I) -> Self {
        let mut b = Bindings::new();
        b.extend(iter);
        b
    }
}

impl IntoIterator for Bindings {
    type Item = (Arc<str>, BindingValue);
    type IntoIter = std::vec::IntoIter<(Arc<str>, BindingValue)>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl<'a> IntoIterator for &'a Bindings {
    type Item = (&'a Arc<str>, &'a BindingValue);
    type IntoIter = std::iter::Map<
        std::slice::Iter<'a, (Arc<str>, BindingValue)>,
        fn(&'a (Arc<str>, BindingValue)) -> (&'a Arc<str>, &'a BindingValue),
    >;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter().map(|(k, v)| (k, v))
    }
}

/// 順序によらず、同じ変数に同じ値が入っていれば等しい（`HashMap` と同じ意味）。
impl PartialEq for Bindings {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().all(|(k, v)| other.get(k) == Some(v))
    }
}

impl std::fmt::Debug for Bindings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}
