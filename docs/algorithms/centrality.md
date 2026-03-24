---
title: PageRank / 中心性指標
parent: グラフアルゴリズム
nav_order: 3
---

# PageRank / 中心性指標

グラフ中心性指標はノードの重要度や影響力を定量的に評価します。MaharitDB は PageRank、媒介中心性（Betweenness）、近接中心性（Closeness）を内蔵しています。

## PageRank

PageRank はウェブページのランキングアルゴリズムとして有名ですが、ソーシャルグラフや知識グラフにも広く応用されます。多くのリンク（エッジ）を受け取るノード、特に重要なノードから多くのリンクを受け取るノードほど高いスコアを持ちます。

### Cypher での使用

```cypher
-- PageRank を計算してスコアの高いノードを返す
CALL db.pageRank(
  "FOLLOWS",   -- エッジタイプ
  0.85,        -- ダンピングファクター（通常 0.85）
  20           -- 反復回数
)
YIELD node, score
RETURN node.name, score
ORDER BY score DESC
LIMIT 10
```

### Rust API での使用

```rust
use maharit_core::{Graph, algorithms};

fn main() {
    let mut graph = Graph::new();
    // グラフを構築...

    let scores = algorithms::page_rank(&graph, "FOLLOWS", 0.85, 20);
    let mut ranked: Vec<_> = scores.iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap());

    for (node_id, score) in ranked.iter().take(10) {
        let node = graph.get_node(**node_id).unwrap();
        println!("{}: {:.4}", node.property("name"), score);
    }
}
```

### ダンピングファクター

ダンピングファクター（d）は、ランダムサーファーがリンクをたどる確率を表します。`1 - d` の確率でランダムなノードに移動します。通常は `0.85` を使用します。

```cypher
-- 収束しやすいダンピングファクターと反復回数
CALL db.pageRank("LINKS", 0.85, 50)
YIELD node, score
RETURN node.name, score
ORDER BY score DESC
```

## 媒介中心性（Betweenness Centrality）

最短経路上に多く登場するノードが高いスコアを持ちます。情報の「橋渡し」となるノードを特定するのに有用です。

### 計算式

```
BC(v) = Σ_{s≠v≠t} (σ_st(v) / σ_st)
```

- `σ_st`: s から t への最短経路の数
- `σ_st(v)`: s から t への最短経路のうち、v を通る経路の数

### Cypher での使用

```cypher
CALL db.betweennessCentrality("KNOWS")
YIELD node, score
RETURN node.name, score
ORDER BY score DESC
LIMIT 10
```

### ユースケース

- ソーシャルグラフでインフルエンサーの特定
- ネットワークのボトルネックの検出
- 情報の伝播経路の分析

## 近接中心性（Closeness Centrality）

他のすべてのノードへの平均距離が短いノードが高いスコアを持ちます。情報をすばやく全体に広める能力を表します。

### 計算式

```
CC(v) = (n - 1) / Σ_{u≠v} d(u, v)
```

- `n`: ノード数
- `d(u, v)`: u と v の間の最短距離

### Cypher での使用

```cypher
CALL db.closenessCentrality("ROAD")
YIELD node, score
RETURN node.name, score
ORDER BY score DESC
LIMIT 10
```

### ユースケース

- 施設配置の最適化（倉庫、病院の立地）
- ネットワークの中心的なハブの特定
- 伝染病のシードノード選定

## 次数中心性（Degree Centrality）

最も単純な中心性指標で、接続するエッジ数（次数）を使用します。

```cypher
-- 出次数（FOLLOWS 先の数）
MATCH (n:Person)
RETURN n.name, count { (n)-[:FOLLOWS]->(:Person) } AS out_degree
ORDER BY out_degree DESC
LIMIT 10

-- 入次数（フォロワー数）
MATCH (n:Person)
RETURN n.name, count { (:Person)-[:FOLLOWS]->(n) } AS in_degree
ORDER BY in_degree DESC
LIMIT 10
```

## 中心性指標の比較

| 指標 | 計算量 | 適したユースケース |
|------|--------|-----------------|
| PageRank | O(k × E)（k は反復回数） | ウェブランキング、ソーシャル影響力 |
| 媒介中心性 | O(V × E) | ブリッジノードの特定 |
| 近接中心性 | O(V × (V + E)) | 最も中心的なノードの特定 |
| 次数中心性 | O(E) | 人気度、活動量の評価 |

## 全中心性指標の一括計算

```cypher
CALL db.centrality.all("KNOWS")
YIELD node, pagerank, betweenness, closeness, degree
RETURN node.name, pagerank, betweenness, closeness, degree
ORDER BY pagerank DESC
LIMIT 20
```
