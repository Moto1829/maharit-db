# グラフアルゴリズムの並列化（Rayon）

## 概要

`algorithms.rs` の PageRank・Betweenness Centrality・Closeness Centrality は
すべて逐次実行。各ノードを始点とした処理が独立しているため、
`rayon` の `par_iter()` を追加するだけでほぼ線形にスケールする。

## 現状の問題

```rust
// betweenness_centrality: 各ノードを始点としたBFSが完全に独立
for &source in &nodes {          // N回の独立した処理
    let mut predecessors = ...;
    // BFS ...
    betweenness[node] += delta[node];
}

// closeness_centrality: 同様に各ノード独立
for &node in nodes {
    let distances = bfs_distances(graph, node);  // 独立
    ...
}
```

## 実装内容

### Betweenness Centrality（効果大・実装容易）

- [ ] `rayon` クレートを `maharit-core/Cargo.toml` に追加
- [ ] 外側の `for &source in &nodes` ループを `par_iter()` に変更
- [ ] 各スレッドがローカルな `HashMap<NodeId, f64>` を持ち、最後に合算
  ```rust
  let partial: Vec<HashMap<NodeId, f64>> = nodes
      .par_iter()
      .map(|&source| {
          let mut local_betweenness = HashMap::new();
          // BFS + 逆伝播
          local_betweenness
      })
      .collect();
  // 合算
  for partial_map in partial {
      for (k, v) in partial_map { *betweenness.entry(k).or_insert(0.0) += v; }
  }
  ```
- [ ] グラフは `&Graph`（読み取り専用）で共有 → データ競合なし

### Closeness Centrality（同様に並列化）

- [ ] `nodes.par_iter().map(|&node| { bfs_distances(...) })` に変更

### PageRank（イテレーション内を並列化）

- [ ] `new_scores` の計算部分を `par_iter()` で並列化
- [ ] ダングリングノードの `sum` を `par_iter().sum()` に変更
- [ ] 注意: イテレーション間は依存あり（並列化はイテレーション内のみ）

### 小規模グラフの安全策

- [ ] ノード数が閾値（例: 500）未満の場合は逐次処理にフォールバック
  （Rayon のオーバーヘッドが利得を上回る場合がある）

## 期待効果

| アルゴリズム | 期待倍率（8コア） |
|------------|--------------|
| Betweenness Centrality | **3〜5倍** |
| Closeness Centrality | 2〜4倍 |
| PageRank | 1.5〜3倍 |

## 対象クレート

`maharit-core`（`rayon` を新規追加）
