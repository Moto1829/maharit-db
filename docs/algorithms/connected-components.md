---
title: 連結成分
parent: グラフアルゴリズム
nav_order: 4
---

# 連結成分

グラフを「互いにつながっているノードのまとまり」に分けます。`maharit-core` クレートの Rust API で、`Graph` 全体に対して計算します。

!!! note "Cypher からは使えません"
    連結成分・強連結成分・ラベル伝播は Rust API だけです。Cypher のプロシージャ（`CALL …`）としては用意していません。どれもリレーションシップの型は区別しません。

## 弱連結成分（Connected Components）

リレーションシップの向きを無視して、つながっているノードを 1 つの成分にまとめます。

```rust
use maharit_core::{Graph, connected_components};

fn main() {
    let mut graph = Graph::new();
    let n: Vec<_> = (0..5).map(|_| graph.create_node("Person")).collect();
    graph.create_edge(n[0], n[1], "KNOWS").unwrap();
    graph.create_edge(n[2], n[1], "KNOWS").unwrap(); // 向きは問わない
    graph.create_edge(n[3], n[4], "KNOWS").unwrap();

    let components = connected_components(&graph);
    println!("{} components", components.len()); // 2
    for (i, component) in components.iter().enumerate() {
        println!("component {i}: {} nodes", component.len());
    }
}
```

戻り値は `Vec<HashSet<NodeId>>` で、成分ごとのノード ID の集合です。どのリレーションシップにもつながっていないノードは、それだけで 1 つの成分になります。

用途の例:

- 成分の数が 1 なら、グラフ全体がつながっている
- 大きさが 1 の成分は孤立したノード
- 成分の大きさの分布で、データの偏りやつながりの欠落に気付ける

## 強連結成分（Strongly Connected Components）

リレーションシップの向きに沿って、互いに行き来できるノードのまとまりです（Kosaraju のアルゴリズム）。たとえば A → B → C → A は 1 つの強連結成分になりますが、A → B だけなら A と B は別の成分です。

```rust
let scc = maharit_core::strongly_connected_components(&graph);
// 2 ノード以上の成分は、循環（相互参照）を含む
let cyclic: Vec<_> = scc.iter().filter(|c| c.len() > 1).collect();
```

用途の例: 相互にフォローし合うグループ、循環参照している依存関係の検出。

## ラベル伝播法（コミュニティ検出）

各ノードが、隣のノード（向きは問わない）で最も多いコミュニティ ID を採用することを繰り返して、密につながったまとまり（コミュニティ）を見つけます。

```rust
let communities = maharit_core::label_propagation(&graph, 50); // 最大 50 回反復
// communities: HashMap<NodeId, u64>（ノード → コミュニティ ID）
```

- 最初は各ノードが自分の ID をコミュニティ ID として持ちます
- 同数の場合は小さい ID を選ぶので、結果は毎回同じになります
- 変化が無くなるか、反復の上限に達したら終わります

## 結果をグラフに書き戻す

Cypher で成分ごとに集計したいときは、計算結果をプロパティとして書き戻します。

```rust
for (i, component) in maharit_core::connected_components(&graph).iter().enumerate() {
    for &id in component {
        graph.get_node_mut(id).unwrap().set_property("component", i as i64);
    }
}
```

```cypher
// 書き戻した後: 成分ごとのノード数
MATCH (n)
RETURN n.component AS component, count(n) AS size
ORDER BY size DESC
```
