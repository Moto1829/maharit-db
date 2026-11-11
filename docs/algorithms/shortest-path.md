---
title: 最短経路
parent: グラフアルゴリズム
nav_order: 1
---

# 最短経路

ホップ数が最小の経路は Cypher の `shortestPath` / `allShortestPaths` で、リレーションシップのプロパティを重みにした最短経路は Rust API（`maharit-core` の `Dijkstra` / `AStar`）で求めます。

## Cypher: shortestPath / allShortestPaths

幅優先探索（BFS）で、ホップ数が最小の経路を返します。

```cypher
// 最短経路を 1 つ
MATCH p = shortestPath((a:Person {name: "Alice"})-[:KNOWS*]-(b:Person {name: "Charlie"}))
RETURN [n IN nodes(p) | n.name] AS names, length(p) AS hops
```

```cypher
// ホップ数が最小の経路をすべて
MATCH p = allShortestPaths((a:Person {name: "Alice"})-[:KNOWS*]-(b:Person {name: "Charlie"}))
RETURN [n IN nodes(p) | n.name] AS names
```

```cypher
// 向きを指定し、最大 5 ホップまで
MATCH p = shortestPath((a:City {name: "Tokyo"})-[:ROAD*..5]->(b:City {name: "Osaka"}))
RETURN [c IN nodes(p) | c.name] AS cities, length(p) AS hops
```

- `-[:KNOWS*]-` は向きを問わず、`-[:ROAD*]->` は矢印の向きだけを辿ります
- 最小のホップ数は 0 または 1 にしてください（`*2..` のような下限は使えません）
- 経路が無ければ行を返しません

## Rust API: 重み付き最短経路（Dijkstra）

リレーションシップのプロパティ（例: `distance`）を重みにした最短経路は、Cypher では求められません。Rust API の `Dijkstra` を使います。重みは 0 以上である必要があります（負の重みには対応していません）。

```rust
use maharit_core::{Dijkstra, Graph, NodeId};

fn city(graph: &mut Graph, name: &str) -> NodeId {
    let id = graph.create_node("City");
    graph.get_node_mut(id).unwrap().set_property("name", name);
    id
}

fn road(graph: &mut Graph, from: NodeId, to: NodeId, km: f64) {
    let id = graph.create_edge(from, to, "ROAD").unwrap();
    graph.get_edge_mut(id).unwrap().set_property("distance", km);
}

fn main() {
    let mut graph = Graph::new();
    let tokyo = city(&mut graph, "Tokyo");
    let nagoya = city(&mut graph, "Nagoya");
    let osaka = city(&mut graph, "Osaka");
    road(&mut graph, tokyo, nagoya, 350.0);
    road(&mut graph, nagoya, osaka, 190.0);
    road(&mut graph, tokyo, osaka, 600.0);

    // ホップ数が最小の経路（BFS）: Tokyo → Osaka
    let path = maharit_core::shortest_path(&graph, tokyo, osaka).unwrap();
    println!("hops: {}", path.len() - 1); // 1

    // distance の合計が最小の経路: Tokyo → Nagoya → Osaka（540）
    let dijkstra = Dijkstra::with_weight_property(&graph, "distance");
    let best = dijkstra.find_path(tokyo, osaka).unwrap();
    println!("nodes: {:?}, total: {}", best.nodes, best.total_weight);

    // 1 つの始点から全ノードへの距離
    let distances = dijkstra.distances_from(tokyo);
    println!("{:?}", distances.get(&osaka)); // Some(540.0)
}
```

- `Dijkstra::new(&graph)` はすべての重みを 1 として扱います。重みのプロパティが無いリレーションシップも 1 になります
- `Dijkstra::with_weight_fn(&graph, |edge| …)` で重みを計算する関数を渡せます
- 目的地までの距離の見積もり（ヒューリスティック）がある場合は `AStar::new(&graph, |from, to| …)` を使うと、探索するノードが減ります
- 結果の `WeightedPath` は `nodes`（ノード ID の列）・`edges`（リレーションシップ ID の列）・`total_weight` を持ちます

## 性能の目安

- 重みの無い最短経路は BFS で、辿るリレーションシップの数に比例した時間で求まります
- 可変長パターンのホップ数に上限（`*..N`）を付けると、探索の範囲を抑えられます
- `allShortestPaths` は最短経路が多いグラフでは結果が大きくなります
