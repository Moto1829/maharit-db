---
title: 全経路探索
parent: グラフアルゴリズム
nav_order: 2
---

# 全経路探索

`all_paths` は指定した 2 ノード間の、最大ホップ数以内のすべての経路を DFS（深さ優先探索）バックトラッキングで列挙します。

## Cypher での使用

```cypher
-- Alice から Bob への最大 4 ホップ以内のすべての経路
MATCH p = (a:Person {name: "Alice"})-[:KNOWS*1..4]->(b:Person {name: "Bob"})
RETURN p, length(p) AS hops
ORDER BY hops ASC

-- すべての経路の長さを確認
MATCH p = (a:Person {name: "Alice"})-[:KNOWS*1..4]->(b:Person {name: "Bob"})
RETURN length(p) AS hops, count(*) AS path_count
ORDER BY hops
```

## CALL db.allPaths()

組み込みのプロシージャで全経路を取得します。

```cypher
CALL db.allPaths(
  "Alice",    -- 始点ノードの識別子
  "Bob",      -- 終点ノードの識別子
  "KNOWS",    -- エッジタイプ
  4           -- 最大ホップ数
)
YIELD path, length
RETURN path, length
ORDER BY length ASC
```

## Rust API での使用

```rust
use maharit_core::{Graph, traversal};

fn main() {
    let mut graph = Graph::new();

    // グラフを構築
    let alice = graph.add_node("Person", [("name", "Alice")]);
    let bob = graph.add_node("Person", [("name", "Bob")]);
    let charlie = graph.add_node("Person", [("name", "Charlie")]);
    let dave = graph.add_node("Person", [("name", "Dave")]);

    graph.add_edge(alice, bob, "KNOWS", []);
    graph.add_edge(alice, charlie, "KNOWS", []);
    graph.add_edge(charlie, bob, "KNOWS", []);
    graph.add_edge(bob, dave, "KNOWS", []);
    graph.add_edge(charlie, dave, "KNOWS", []);

    // alice から dave への最大 3 ホップの全経路
    let paths = traversal::all_paths(&graph, alice, dave, 3);

    println!("Found {} paths:", paths.len());
    for (i, path) in paths.iter().enumerate() {
        let names: Vec<String> = path.iter()
            .map(|&node_id| graph.get_node(node_id).unwrap().property("name").to_string())
            .collect();
        println!("  Path {}: {} (hops: {})", i + 1, names.join(" -> "), path.len() - 1);
    }
}
```

出力例：

```
Found 3 paths:
  Path 1: Alice -> Bob -> Dave (hops: 2)
  Path 2: Alice -> Charlie -> Bob -> Dave (hops: 3)
  Path 3: Alice -> Charlie -> Dave (hops: 2)
```

## エッジタイプのフィルタリング

```cypher
-- 特定のエッジタイプのみを使用した経路
MATCH p = (a:Location {name: "A"})-[:ROAD|FERRY*1..5]->(b:Location {name: "B"})
RETURN p, length(p) AS hops

-- エッジタイプを問わない全経路
MATCH p = (a:Node {id: 1})-[*1..3]->(b:Node {id: 10})
RETURN p, length(p)
```

## サイクル検出への注意

`all_paths` はシンプルパス（同じノードを 2 度通らない経路）のみを返します。循環経路は含まれません。

```cypher
-- シンプルパスのみ（デフォルト動作）
MATCH p = (a:Person {name: "Alice"})-[:KNOWS*1..5]->(b:Person {name: "Alice"})
RETURN p  -- 結果なし（シンプルパスの場合 Alice に戻れない）
```

## 経路の統計分析

```cypher
-- 全経路の統計を集計
MATCH p = (a:Person {name: "Alice"})-[:KNOWS*1..4]->(b:Person)
WHERE b.name <> "Alice"
RETURN
  b.name AS target,
  count(p) AS path_count,
  min(length(p)) AS min_hops,
  max(length(p)) AS max_hops
ORDER BY min_hops, path_count DESC
```

## パフォーマンスの注意

全経路探索は組み合わせ爆発が起こりやすいアルゴリズムです。

- ホップ数の上限を必ず設定してください
- 密なグラフでは経路数が指数的に増加します
- 経路数が多い場合は `LIMIT` を使用して制限してください

```cypher
-- 最初の 100 経路のみ取得
MATCH p = (a:Person {name: "Alice"})-[:KNOWS*1..4]->(b:Person)
RETURN p
LIMIT 100
```

## ユースケース

- **ソーシャルグラフの到達性分析**: 何人を介して知り合いになれるか
- **ネットワーク冗長性の確認**: 単一障害点の検出
- **依存関係の全ルート列挙**: ソフトウェアパッケージの依存ツリー
- **物流ルート分析**: 可能なすべての配送ルートの列挙
