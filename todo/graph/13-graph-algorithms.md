# グラフ分析アルゴリズム

**Status**: Completed

## 概要
グラフ分析のための高度なアルゴリズムを実装する。

## 実装内容

### 中心性指標
- [x] 次数中心性（Degree Centrality）
- [x] 近接中心性（Closeness Centrality）
- [x] 媒介中心性（Betweenness Centrality）

### PageRank
- [x] 基本実装（べき乗法）
- [x] ダンピングファクター設定
- [x] 収束判定

### コミュニティ検出
- [x] 連結成分の抽出
- [x] 強連結成分（SCC）
- [x] ラベル伝播法（将来的）

### サイクル検出
- [x] 有向グラフのサイクル検出
- [x] トポロジカルソート

### API
```rust
let centrality = DegreeCentrality::compute(&graph);
let pr = pagerank(&graph, &PageRankConfig::default());
let components = connected_components(&graph);
let sccs = strongly_connected_components(&graph);
let has_cycle = has_cycle(&graph);
let sorted = topological_sort(&graph);
```

## 対象クレート
`maharit-core`
