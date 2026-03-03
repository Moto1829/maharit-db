# クエリエンジン: MATCH 候補フィルタリングの並列化

## 概要

`MATCH` のパターンマッチングで候補ノードを絞り込む処理が逐次ループ。
グラフは読み取り専用参照（`&Graph`）のため、
`rayon` の `par_iter()` で安全に並列化できる。

## 現状の問題

```rust
// executor.rs: 候補ノードの逐次フィルタリング
fn match_node_pattern(graph: &Graph, pattern: &NodePattern) -> Vec<NodeId> {
    graph.nodes()
        .filter(|n| {
            pattern.labels.iter().all(|l| n.has_label(l))
                && match_properties(n, &pattern.properties)
        })
        .map(|n| n.id)
        .collect()   // 逐次処理
}
```

大規模グラフで全ノードスキャンが発生する場合に顕著。

## 実装内容

- [ ] `graph.nodes()` を `par_iter()` に変更
  ```rust
  fn match_node_pattern(graph: &Graph, pattern: &NodePattern) -> Vec<NodeId> {
      graph.nodes()
          .par_iter()
          .filter(|n| {
              pattern.labels.iter().all(|l| n.has_label(l))
                  && match_properties(n, &pattern.properties)
          })
          .map(|n| n.id)
          .collect()
  }
  ```
- [ ] 複数バインディングに対する次パターンのマッチングも並列化
  ```rust
  let next_bindings: Vec<Bindings> = all_bindings
      .par_iter()
      .flat_map(|binding| match_pattern_with_binding(pattern, binding))
      .collect();
  ```
- [ ] インデックスが使える場合（`NodeIndexSeek`）は並列化不要・スキップ
  （インデックスヒットは既に少数ノードのみ処理するため）
- [ ] `rayon` を `maharit-query/Cargo.toml` に追加

## 注意

- `Executor` が `&mut Graph` を持っているが、フィルタリングは読み取りのみ
- `Arc<RwLock<Graph>>` モデルと組み合わせる場合、`read()` ロック下で並列化
- バインディング数が少ない場合は逐次処理のほうが速い可能性あり（閾値チェック推奨）

## 期待効果

- 大規模グラフでの全ノードスキャン: **2〜4倍**
- 複数バインディングのパターン結合: **2〜3倍**
- `LIMIT` 付きクエリは 59（遅延評価）と組み合わせると特に効果的

## 依存

- タスク 58（DashMap/RwLock 改善）と組み合わせると読み取り並列性がさらに高まる

## 対象クレート

`maharit-query`
