# クエリエンジン: クエリ結果のプロパティ共有化

**Status**: Completed

## 概要

クエリ結果の `Value::NodeData` がノードのプロパティ `HashMap` を毎回フルコピーしている。
`Arc` による共有参照化でコピーコストを削減する。

## 現状の問題

```rust
pub enum Value {
    NodeData {
        id: NodeId,
        labels: Vec<String>,
        properties: HashMap<String, PropertyValue>,  // ノードから毎回フルクローン
    },
    // ...
}
```

`MATCH (n:Person) RETURN n` のように多数のノードを返す場合、
各ノードのプロパティ HashMap を全てコピーするためメモリと時間を消費する。

## 実装内容

- [x] `Node.properties` を `Arc<HashMap<String, PropertyValue>>` に変更
- [x] `Value::NodeData.properties` も `Arc<HashMap<String, PropertyValue>>` に変更
- [x] ノード返却時は `Arc::clone()` のみ（実データのコピーなし）
- [x] プロパティ更新時（`SET`）は `Arc::make_mut()` で CoW（Copy-on-Write）
- [x] `Value::EdgeData` も同様に対応
- [x] 既存の全テストが通ること

## 期待効果

- 大量ノード返却クエリのメモリ使用量 -80%（プロパティのコピーが参照カウントのみに）
- `RETURN n` 系クエリのスループット向上

## 注意

- `Arc::make_mut()` はプロパティ更新時に自動的にクローンするため、
  更新が少なく読み取りが多いワークロードで最も効果的
- シリアライズ（`persistence.rs`）側でも `Arc` を透過的に扱えるか確認

## 対象クレート

`maharit-core`, `maharit-query`

## Changes

### `crates/maharit-core/src/graph.rs`
- `Node.properties` の型を `HashMap<String, PropertyValue>` から `Arc<HashMap<String, PropertyValue>>` に変更
- `Edge.properties` の型を同様に変更
- `Node::set_property()` と `Node::remove_property()` を `Arc::make_mut()` を使う形に更新
- `Edge::set_property()` と `Edge::remove_property()` を同様に更新
- `create_node_with_labels()`、`create_node_with_id_and_labels()`、`create_edge()` の初期化を `Arc::new(HashMap::new())` に変更

### `crates/maharit-query/src/executor.rs`
- `use std::sync::Arc;` をインポートに追加
- `Value::NodeData.properties` の型を `Arc<HashMap<String, PropertyValue>>` に変更
- `NodeData` 生成箇所（6箇所）で `node.properties.clone()` を `Arc::clone(&node.properties)` に変更
- テストコードで `node.properties.insert(...)` を `node.set_property(...)` に変更（3箇所）

### `crates/maharit-storage/src/transaction.rs`
- `UndoRecord::DeleteNode.properties` の型を `Arc<HashMap<...>>` に変更（Arc clone で整合性を保つ）
- `for (key, value) in properties` を `for (key, value) in properties.iter()` に変更

### `crates/maharit-storage/src/backup.rs`
- WAL replay での直接代入 `node.properties = properties` を `node.properties = std::sync::Arc::new(properties)` に変更
- 同様に `edge.properties = properties` を変更

### `crates/maharit-io/src/graphml_io.rs`
- `for (key, value) in &node.properties` を `for (key, value) in node.properties.iter()` に変更（4箇所）

## Tests

全テストスイートが通過（857テスト）:
- maharit-core: 147 passed
- maharit-io: 20 passed
- maharit-query: 424 passed
- maharit-storage: 171 passed
- maharit-server: 56 passed
- maharit-viz: 35 passed
