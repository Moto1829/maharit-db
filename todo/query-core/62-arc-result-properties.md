# クエリエンジン: クエリ結果のプロパティ共有化

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

- [ ] `Node.properties` を `Arc<HashMap<String, PropertyValue>>` に変更
- [ ] `Value::NodeData.properties` も `Arc<HashMap<String, PropertyValue>>` に変更
- [ ] ノード返却時は `Arc::clone()` のみ（実データのコピーなし）
- [ ] プロパティ更新時（`SET`）は `Arc::make_mut()` で CoW（Copy-on-Write）
- [ ] `Value::EdgeData` も同様に対応
- [ ] 既存の全テストが通ること

## 期待効果

- 大量ノード返却クエリのメモリ使用量 -80%（プロパティのコピーが参照カウントのみに）
- `RETURN n` 系クエリのスループット向上

## 注意

- `Arc::make_mut()` はプロパティ更新時に自動的にクローンするため、
  更新が少なく読み取りが多いワークロードで最も効果的
- シリアライズ（`persistence.rs`）側でも `Arc` を透過的に扱えるか確認

## 対象クレート

`maharit-core`, `maharit-query`
