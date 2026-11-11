---
title: PageRank / 中心性指標
parent: グラフアルゴリズム
nav_order: 3
---

# PageRank / 中心性指標

ノードの重要度を測る指標です。`maharit-core` クレートの Rust API で、`Graph` 全体に対して計算します。

!!! note "Cypher からは使えません"
    中心性の計算は Rust API だけです。Cypher のプロシージャ（`CALL …`）としては用意していません。どの指標も、リレーションシップの型は区別せず、グラフ全体を対象にします。

## PageRank

リンクを多く受けるノード、重要なノードからリンクされるノードほど高いスコアになります。リレーションシップの向き（`(a)-->(b)` は a から b へのリンク）を使います。

```rust
use maharit_core::{Graph, PageRankConfig, pagerank};

fn main() {
    let mut graph = Graph::new();
    let pages: Vec<_> = (0..4).map(|_| graph.create_node("Page")).collect();
    for (from, to) in [(0, 1), (0, 2), (1, 2), (2, 0), (3, 2)] {
        graph.create_edge(pages[from], pages[to], "LINKS").unwrap();
    }

    let result = pagerank(&graph, &PageRankConfig::default());
    println!("iterations: {}, converged: {}", result.iterations, result.converged);

    let mut ranked: Vec<_> = result.scores.iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(a.1));
    for (node, score) in ranked {
        println!("{node}: {score:.4}");
    }
}
```

`PageRankConfig` の項目:

| 項目 | 既定値 | 説明 |
|---|---|---|
| `damping` | `0.85` | ダンピングファクター。リンクを辿り続ける確率 |
| `max_iterations` | `100` | 反復の上限 |
| `tolerance` | `1e-6` | スコアの変化がこれ未満になったら収束とみなす |

出リンクの無いノード（ダングリングノード）のスコアは、全ノードに均等に配ります。

## 媒介中心性（Betweenness Centrality）

ほかのノードどうしを結ぶ最短経路の上に、そのノードがどれだけ現れるかを表します。値が大きいノードは、グループの間をつなぐ「橋渡し役」です。Brandes のアルゴリズムで、リレーションシップの向きに沿った最短経路を数えます。

```rust
let scores = maharit_core::betweenness_centrality(&graph);
```

## 近接中心性（Closeness Centrality）

そのノードからほかのノードへの距離（ホップ数）が短いほど大きくなります。到達できるノードの数を `r`、それらへの距離の合計を `d` として `(r - 1) / d` で、リレーションシップの向きに沿って辿ります。どこにも到達できないノードは 0 です。

```rust
let scores = maharit_core::closeness_centrality(&graph);
```

## 次数中心性（Degree Centrality）

ノードにつながるリレーションシップの数です。

```rust
use maharit_core::DegreeCentrality;

let degree = DegreeCentrality::compute(&graph);
let in_degree = &degree.in_degree;       // 入ってくる数
let out_degree = &degree.out_degree;     // 出ていく数
let total = &degree.total_degree;        // 合計
let normalized = degree.normalized(&graph); // 合計を 2(n-1) で割った値
```

## 指標の比較

| 指標 | 計算量 | 向いている用途 |
|------|--------|-----------------|
| PageRank | O(k × E)（k は反復回数） | ウェブページのランキング、影響力 |
| 媒介中心性 | O(V × E) | 橋渡し役のノードの発見 |
| 近接中心性 | O(V × (V + E)) | 全体の中心にいるノードの発見 |
| 次数中心性 | O(E) | 人気度・活動量 |

ノード数が 500 以上のグラフでは、媒介中心性と近接中心性を並列に計算します（[並列化とパフォーマンス](../advanced/parallelization.md)）。

## 結果をグラフに書き戻す

計算結果は `HashMap<NodeId, f64>` で返ります。Cypher から使うには、プロパティとして書き戻します。

```rust
let scores = maharit_core::betweenness_centrality(&graph);
for (id, score) in scores {
    if let Some(node) = graph.get_node_mut(id) {
        node.set_property("betweenness", score);
    }
}
```

サーバーのデータに対して計算したい場合は、サーバーを停止してから、データファイルを `maharit_storage::PersistentStorage::load_with_schema` で読み込み、計算・書き戻して `save_with_schema` で保存してください（`load` / `save` だとインデックス・制約の定義が失われます）。WAL（`<data>.wal`）が残っていると起動時に再適用されるので、停止時に空になっていることを確認してください。
