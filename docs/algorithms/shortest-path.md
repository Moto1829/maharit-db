---
title: 最短経路
parent: グラフアルゴリズム
nav_order: 1
---

# 最短経路

MaharitDB は BFS（幅優先探索）による重みなし最短経路と、Dijkstra 法による重み付き最短経路の両方をサポートしています。

## BFS による最短経路

重みを考慮しない（ホップ数最小の）経路を返します。

### Cypher での使用

```cypher
-- 最短経路を取得
MATCH p = shortestPath((a:Person {name: "Alice"})-[:KNOWS*]-(b:Person {name: "Charlie"}))
RETURN p, length(p) AS hops
```

### 複数の最短経路

```cypher
-- すべての最短経路を取得
MATCH p = allShortestPaths((a:Person {name: "Alice"})-[:KNOWS*]-(b:Person {name: "Charlie"}))
RETURN p
```

### 方向の指定

```cypher
-- 有向グラフでの最短経路
MATCH p = shortestPath((a:City {name: "Tokyo"})-[:ROAD*]->(b:City {name: "Osaka"}))
RETURN p, length(p) AS hops

-- 無向グラフとして扱う（どちらの方向でも可）
MATCH p = shortestPath((a:Person)-[:KNOWS*]-(b:Person))
WHERE a.name = "Alice" AND b.name = "Eve"
RETURN p
```

## Dijkstra 法による重み付き最短経路

エッジのプロパティを重みとして使用します。

### CALL db.shortestPath.dijkstra()

```cypher
CALL db.shortestPath.dijkstra(
  "Alice",        -- 始点ノードの名前または ID
  "Osaka",        -- 終点ノードの名前または ID
  "ROAD",         -- エッジタイプ
  "distance"      -- 重みプロパティ名
)
YIELD path, total_weight
RETURN path, total_weight
```

### 経路の情報を取得

```cypher
MATCH p = shortestPath((a:City {name: "Tokyo"})-[:ROAD*]->(b:City {name: "Fukuoka"}))
RETURN
  [city IN nodes(p) | city.name] AS cities,
  length(p) AS hops,
  reduce(total = 0, r IN relationships(p) | total + r.distance) AS total_distance
```

## Rust API での使用

```rust
use maharit_core::{Graph, traversal};

fn main() {
    let mut graph = Graph::new();

    // ノードを追加
    let tokyo = graph.add_node("City", [("name", "Tokyo")]);
    let nagoya = graph.add_node("City", [("name", "Nagoya")]);
    let osaka = graph.add_node("City", [("name", "Osaka")]);

    // エッジを追加（距離付き）
    graph.add_edge(tokyo, nagoya, "ROAD", [("distance", 350.0)]);
    graph.add_edge(nagoya, osaka, "ROAD", [("distance", 190.0)]);
    graph.add_edge(tokyo, osaka, "ROAD", [("distance", 500.0)]);

    // BFS 最短経路
    if let Some(path) = traversal::shortest_path(&graph, tokyo, osaka) {
        println!("Shortest path: {:?}", path);
        println!("Hops: {}", path.len() - 1);
    }

    // Dijkstra 最短経路
    if let Some((path, cost)) = traversal::dijkstra(&graph, tokyo, osaka, "distance") {
        println!("Optimal path: {:?}", path);
        println!("Total distance: {}", cost);
    }
}
```

## ホップ数の制限

```cypher
-- 最大 5 ホップ以内の最短経路
MATCH p = shortestPath((a:Person {name: "Alice"})-[:KNOWS*..5]-(b:Person {name: "Bob"}))
RETURN p, length(p) AS hops
```

## 経路の条件付き検索

```cypher
-- 特定の条件を満たすノードを通る最短経路
MATCH p = shortestPath(
  (a:City {name: "Tokyo"})-[:ROAD*]->(b:City {name: "Osaka"})
)
WHERE all(city IN nodes(p) WHERE city.accessible = true)
RETURN p
```

## パフォーマンスの考慮事項

- BFS は重みなしグラフでは最も効率的です
- Dijkstra は重み付きグラフに適していますが、負の重みには対応していません（Bellman-Ford を使用してください）
- ホップ数の制限（`*..N`）を設定することで処理時間を制限できます
- 大規模グラフでの長い経路探索には時間がかかる場合があります
