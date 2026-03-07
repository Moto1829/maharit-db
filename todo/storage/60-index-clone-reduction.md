# インデックス: ラベル文字列の不要なクローンを削減

## 概要

`LabelIndex::add_node_labels()` でラベル文字列が3回以上クローンされており、
ラベル追加のたびに無駄なアロケーションが発生している。
`HashSet` の活用と `Cow` 化で改善する。

## 現状の問題

```rust
pub fn add_node_labels(&mut self, node_id: NodeId, labels: &[&str]) {
    let non_empty: Vec<String> = labels
        .iter()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())       // 1回目のクローン
        .collect();

    for label in &non_empty {
        self.node_labels
            .entry(label.clone())     // 2回目のクローン
            .or_default()
            .insert(node_id);
    }

    let existing = self.node_to_labels.entry(node_id).or_default();
    for label in &non_empty {
        if !existing.contains(label) { // O(N) 線形探索
            existing.push(label.clone()); // 3回目のクローン
        }
    }
}
```

## 実装内容

- [x] `node_to_labels: HashMap<NodeId, Vec<String>>` →
  `HashMap<NodeId, HashSet<String>>` に変更して重複チェックを O(1) に
- [x] `entry(label.clone())` を `entry_ref()` または
  `.raw_entry_mut()` で回避（クローンなしの挿入）
- [x] `get_nodes_by_label()` の戻り値を
  `impl Iterator<Item = NodeId> + '_` に変更（毎回の Vec 作成を廃止）
- [x] `delete_node()` 時の `incoming.retain(|&e| e != edge_id)` を
  `HashSet` で O(1) 削除に変更（`graph.rs` の隣接リスト）
- [x] 既存インデックステストが全て通ること

## 期待効果

- ラベル追加操作のアロケーション -60%
- 重複ラベルチェックが O(N) → O(1)
- ノード削除時のエッジ整理が O(degree) → O(1)

## 対象クレート

`maharit-core`
