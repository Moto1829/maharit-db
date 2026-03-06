# トラバーサル: Vec アロケーションをイテレータ設計に変更

**Status**: Completed

## 概要

`get_outgoing_edges()` / `get_incoming_edges()` が毎回 `Vec` を生成するため、
BFS/DFS のホットパスで大量のアロケーションが発生している。
イテレータを返す設計に変えてアロケーションをゼロにする。

## 現状の問題

```rust
// graph.rs
pub fn get_outgoing_edges(&self, node_id: NodeId) -> Vec<&Edge> {
    self.outgoing_edges
        .get(&node_id)
        .map(|edge_ids| edge_ids.iter().filter_map(|id| self.edges.get(id)).collect())
        .unwrap_or_default()   // BFS/DFS で何度も呼ばれるホットパス
}
```

```rust
// traversal.rs
fn get_neighbors(&self, node_id: NodeId) -> Vec<(NodeId, &'a Edge)> {
    let mut neighbors = Vec::new();   // 毎イテレーションで Vec 作成
    for edge in self.graph.get_outgoing_edges(node_id) { ... }
    neighbors
}
```

## 実装内容

- [x] `get_outgoing_edges()` の戻り値を `impl Iterator<Item = &Edge> + '_` に変更
- [x] `get_incoming_edges()` も同様に変更
- [x] `Traverser::get_neighbors()` をイテレータチェーンで実装
  （`Vec` 作成をなくし、フィルタリング後の要素を直接スタック/キューに push）
- [x] `all_paths_dfs()` で発見パス追加時の `current_path.clone()` を最小化
  （パス長が確定するまで参照のみ保持し、結果追加時のみコピー）
- [x] Dijkstra の `distances` / `previous` を `FxHashMap` または
  `Vec<Option<f64>>` に変更（稠密 ID を前提）
- [x] 既存の traversal テストが全て通ること

## 期待効果

- BFS/DFS 中のアロケーション -50%
- 大規模グラフの経路探索スループット向上

## 依存

- `56-dense-array-graph.md` が完了していると相乗効果が大きい

## 対象クレート

`maharit-core`
