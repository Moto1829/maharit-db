---
title: 並列化とパフォーマンス
parent: 高度なトピック
nav_order: 6
---

# 並列化とパフォーマンス

MaharitDB のグラフアルゴリズムは、大規模グラフに対して自動的に並列処理を有効化します。並列化には [Rayon](https://docs.rs/rayon) を使用しており、データ並列イテレータによってグラフ解析の計算時間を短縮できます。

## 自動並列化の概要

グラフアルゴリズムの並列化は `maharit-core` の `algorithms.rs` に実装されています。アルゴリズムはグラフのノード数を確認し、閾値以上であれば Rayon の並列イテレータ（`par_iter()`）を使用します。閾値未満の場合は逐次処理が選択されます。

```rust
// algorithms.rs より
const PARALLEL_THRESHOLD: usize = 500;
```

### なぜ閾値があるか

Rayon はスレッドプールを使った並列処理を提供しますが、スレッドプールへのタスク配布や同期にはオーバーヘッドがあります。ノード数が少ないグラフでは、並列化によるオーバーヘッドが計算コストを上回り、逐次処理より遅くなる場合があります。

500 ノードを閾値として設定することで、以下を両立しています。

- 小規模グラフ（< 500 ノード）: スレッドプールのオーバーヘッドを避け、逐次処理で高速に完了する
- 大規模グラフ（>= 500 ノード）: 並列処理によりマルチコアCPUを最大限に活用する

## 各アルゴリズムの並列化状況

| アルゴリズム | 並列化 | 並列化される処理 | 備考 |
|---|---|---|---|
| Closeness Centrality | あり | 各ソースノードからの BFS 計算 | ノード数 >= 500 で `par_iter()` |
| Betweenness Centrality | あり | Brandes アルゴリズムの外ループ | ノード数 >= 500 で `par_iter()` |
| PageRank | 一部あり | ダングリングノードのスコア合計 | ノード数 >= 500 でダングリング sum のみ並列 |
| Degree Centrality | なし | — | 逐次処理（エッジ走査のみ） |
| Connected Components | なし | — | 逐次 BFS（依存関係あり） |
| Strongly Connected Components | なし | — | Kosaraju アルゴリズム（逐次 DFS） |
| Label Propagation | なし | — | 反復依存のため逐次処理 |
| Cycle Detection | なし | — | 逐次 DFS |
| Topological Sort | なし | — | Kahn アルゴリズム（逐次） |
| Shortest Path (Dijkstra) | なし | — | 逐次優先度付きキュー |

### 並列化の詳細

**Closeness Centrality** は各ノードからの BFS 計算が独立しているため、全ノードを並列処理できます。

```rust
if n >= PARALLEL_THRESHOLD {
    node_ids.par_iter().map(compute_one).collect()
} else {
    node_ids.iter().map(compute_one).collect()
}
```

**Betweenness Centrality** は Brandes アルゴリズムの外ループ（ソースノードごとの部分中心性計算）を並列化します。各スレッドがローカルな部分中心性マップを生成し、最後にマージします。`Graph` への参照が共有不変参照（`&Graph`）のみであるため安全に並列実行できます。

```rust
let partial_maps: Vec<HashMap<NodeId, f64>> = if n >= PARALLEL_THRESHOLD {
    nodes
        .par_iter()
        .map(|&source| brandes_single(graph, &nodes, source))
        .collect()
} else {
    nodes
        .iter()
        .map(|&source| brandes_single(graph, &nodes, source))
        .collect()
};
```

**PageRank** はエッジ貢献の集積（`HashMap` への書き込み）が逐次実行を要するため、ダングリングノード（出次数 = 0 のノード）のスコア合計のみを並列化します。

```rust
let dangling_sum: f64 = if use_parallel {
    dangling_nodes.par_iter().map(|id| scores[id]).sum()
} else {
    dangling_nodes.iter().map(|id| scores[id]).sum()
};
```

**Label Propagation** は各イテレーションの更新が前イテレーションの結果に依存するため、並列化されていません。

## スレッド数の制御

Rayon はデフォルトで論理 CPU 数に等しいスレッド数のグローバルスレッドプールを使用します。スレッド数は環境変数で制御できます。

```bash
# スレッド数を 4 に制限する
RAYON_NUM_THREADS=4 maharit-server --config config.toml
```

Rayon のグローバルスレッドプールをプログラムから設定する場合:

```rust
rayon::ThreadPoolBuilder::new()
    .num_threads(4)
    .build_global()
    .unwrap();
```

グローバルスレッドプールの設定は一度だけ有効です。複数回呼ぶとエラーになります。

## パフォーマンス特性

### 小規模グラフ（ノード数 < 500）

逐次処理が使用されます。Rayon のスレッドプールへのタスク配布オーバーヘッドがないため、ほぼすべてのアルゴリズムでメモリアクセスが最適化されます。

### 大規模グラフ（ノード数 >= 500）

Rayon の並列イテレータが有効になります。コア数が多いほどスループットが向上しますが、以下の点に注意してください。

- **Closeness Centrality**: ノード数に対してほぼ線形なスケーリングが期待できます（各 BFS が独立）
- **Betweenness Centrality**: 部分結果のマージフェーズに O(N) のシリアル処理があります
- **PageRank**: 並列化はダングリングサム計算の一部のみのため、並列化による恩恵は限定的です

### メモリ使用量

Betweenness Centrality の並列実装は、各スレッドがローカルな `HashMap<NodeId, f64>` を保持するため、逐次実装と比較してスレッド数に比例したメモリを追加消費します。大規模グラフで全コアを使用する場合は、この点を考慮してください。
