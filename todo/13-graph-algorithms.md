# グラフ分析アルゴリズム

## 概要
グラフ分析のための高度なアルゴリズムを実装する。

## 実装内容

### 中心性指標
- [ ] 次数中心性（Degree Centrality）
- [ ] 近接中心性（Closeness Centrality）
- [ ] 媒介中心性（Betweenness Centrality）

### PageRank
- [ ] 基本実装（べき乗法）
- [ ] ダンピングファクター設定
- [ ] 収束判定

### コミュニティ検出
- [ ] 連結成分の抽出
- [ ] 強連結成分（SCC）
- [ ] ラベル伝播法（将来的）

### サイクル検出
- [ ] 有向グラフのサイクル検出
- [ ] トポロジカルソート

### API
```rust
let pr = graph.pagerank(damping: 0.85, max_iter: 100);
let communities = graph.connected_components();
```

## 対象クレート
新規 `maharit-algo` または `maharit-core`
