---
title: 全経路探索
parent: グラフアルゴリズム
nav_order: 2
---

# 全経路探索

2 つのノードの間の経路を、ホップ数の上限までですべて列挙します。Cypher では可変長パターンで、Rust API では `all_paths` で求めます。どちらも同じノードを 2 度通らない経路（シンプルパス）だけを返します。

## Cypher: 可変長パターン

```cypher
// Alice から Bob への 4 ホップ以内の経路をすべて
MATCH p = (a:Person {name: "Alice"})-[:KNOWS*1..4]->(b:Person {name: "Bob"})
RETURN [n IN nodes(p) | n.name] AS names, length(p) AS hops
ORDER BY hops ASC
```

```cypher
// ホップ数ごとの経路の数
MATCH p = (a:Person {name: "Alice"})-[:KNOWS*1..4]->(b:Person {name: "Bob"})
RETURN length(p) AS hops, count(*) AS path_count
ORDER BY hops
```

```cypher
// リレーションシップの型を問わない
MATCH p = (a:Node {id: 1})-[*1..3]->(b:Node {id: 10})
RETURN p, length(p)
```

リレーションシップの型を複数指定する書き方（`[:ROAD|FERRY*]`）にはまだ対応していません（query-clauses/164）。

## Rust API: all_paths

`maharit_core::all_paths(&graph, from, to, max_depth)` は、深さ優先探索（DFS）とバックトラックで経路を列挙します。リレーションシップの向きに沿って辿り、型は区別しません。`max_depth` が `None` なら上限なしです。

```rust
use maharit_core::{Graph, NodeId, PropertyValue, all_paths};

fn person(graph: &mut Graph, name: &str) -> NodeId {
    let id = graph.create_node("Person");
    graph.get_node_mut(id).unwrap().set_property("name", name);
    id
}

fn main() {
    let mut graph = Graph::new();
    let alice = person(&mut graph, "Alice");
    let bob = person(&mut graph, "Bob");
    let charlie = person(&mut graph, "Charlie");
    let dave = person(&mut graph, "Dave");
    for (from, to) in [(alice, bob), (alice, charlie), (charlie, bob), (bob, dave), (charlie, dave)] {
        graph.create_edge(from, to, "KNOWS").unwrap();
    }

    let paths = all_paths(&graph, alice, dave, Some(3));
    println!("Found {} paths:", paths.len());
    for path in &paths {
        let names: Vec<String> = path
            .nodes
            .iter()
            .map(|id| match graph.get_node(*id).and_then(|n| n.get_property("name")) {
                Some(PropertyValue::String(s)) => s.clone(),
                _ => id.to_string(),
            })
            .collect();
        println!("  {} (hops: {})", names.join(" -> "), path.len() - 1);
    }
}
```

出力例（経路の順序は実行ごとに変わることがあります）:

```text
Found 3 paths:
  Alice -> Bob -> Dave (hops: 2)
  Alice -> Charlie -> Bob -> Dave (hops: 3)
  Alice -> Charlie -> Dave (hops: 2)
```

## サイクル検出への注意

可変長パターンも `all_paths` も、シンプルパス（同じノードを 2 度通らない経路）だけを返します。始点に戻る経路は含まれません。

```cypher
// シンプルパスのみ（デフォルト動作）
MATCH p = (a:Person {name: "Alice"})-[:KNOWS*1..5]->(b:Person {name: "Alice"})
RETURN p  // 結果なし（シンプルパスの場合 Alice に戻れない）
```

## 経路の統計分析

```cypher
// 全経路の統計を集計
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
// 最初の 100 経路のみ取得
MATCH p = (a:Person {name: "Alice"})-[:KNOWS*1..4]->(b:Person)
RETURN p
LIMIT 100
```

## ユースケース

- **ソーシャルグラフの到達性分析**: 何人を介して知り合いになれるか
- **ネットワーク冗長性の確認**: 単一障害点の検出
- **依存関係の全ルート列挙**: ソフトウェアパッケージの依存ツリー
- **物流ルート分析**: 可能なすべての配送ルートの列挙
