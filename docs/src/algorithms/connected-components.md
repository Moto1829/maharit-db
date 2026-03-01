# 連結成分

連結成分アルゴリズムはグラフを互いに到達可能なノードのグループ（クラスター）に分割します。

## 弱連結成分（Connected Components）

無向グラフとしてグラフを扱い、連結したノードのグループを検出します。Union-Find（Disjoint Set Union）アルゴリズムを使用します。

### Cypher での使用

```cypher
CALL db.connectedComponents("KNOWS")
YIELD component_id, nodes, node_count
RETURN component_id, [n IN nodes | n.name] AS member_names, node_count
ORDER BY node_count DESC
```

出力例：

```
+------------------+------------------------+-----------+
| component_id     | member_names           | node_count |
+------------------+------------------------+-----------+
| 1                | ["Alice", "Bob", "Charlie"] | 3    |
| 2                | ["Dave", "Eve"]         | 2         |
| 3                | ["Frank"]               | 1         |
+------------------+------------------------+-----------+
```

### 連結成分のラベリング

各ノードに所属するコンポーネント ID を付与します。

```cypher
CALL db.connectedComponents("KNOWS")
YIELD component_id, nodes
FOREACH (n IN nodes |
  SET n.component_id = component_id
)
```

その後、コンポーネント ID でクエリできます：

```cypher
-- 特定のコンポーネントのノードを取得
MATCH (n:Person {component_id: 1})
RETURN n.name

-- コンポーネントをまたぐクエリ
MATCH (a:Person), (b:Person)
WHERE a.component_id <> b.component_id
RETURN a.name, b.name, "separated" AS status
LIMIT 10
```

## 強連結成分（Strongly Connected Components）

有向グラフで、互いに到達可能（双方向に経路が存在する）なノードのグループを検出します。Tarjan 法または Kosaraju 法を使用します。

### Cypher での使用

```cypher
CALL db.stronglyConnectedComponents("FOLLOWS")
YIELD component_id, nodes, node_count
RETURN component_id, [n IN nodes | n.name] AS members, node_count
ORDER BY node_count DESC
LIMIT 10
```

### 有向グラフでの意味

強連結成分は、グラフ内でサイクルを形成するノードのグループです。

```
A → B → C → A  (A, B, C は同じ強連結成分)
D → A           (D は別の強連結成分)
```

### ユースケース

- **循環依存の検出**: パッケージ管理でのサイクル検出
- **ウェブクローリング**: 相互リンクするページのクラスタリング
- **ソーシャルグラフ**: 相互フォローのコミュニティ検出

## Rust API での使用

```rust
use maharit_core::{Graph, algorithms};

fn main() {
    let graph = Graph::new();
    // グラフを構築...

    // 弱連結成分
    let components = algorithms::connected_components(&graph, "KNOWS");
    println!("Number of components: {}", components.len());
    for (id, nodes) in components.iter().enumerate() {
        println!("Component {}: {} nodes", id + 1, nodes.len());
    }

    // 強連結成分
    let scc = algorithms::strongly_connected_components(&graph, "FOLLOWS");
    println!("Number of SCCs: {}", scc.len());
    for (id, nodes) in scc.iter().enumerate() {
        if nodes.len() > 1 {
            println!("SCC {}: {} nodes (cycle exists)", id + 1, nodes.len());
        }
    }
}
```

## コンポーネントサイズの分布分析

```cypher
-- コンポーネントサイズの分布を確認
CALL db.connectedComponents("KNOWS")
YIELD component_id, node_count
RETURN node_count, count(*) AS num_components
ORDER BY node_count DESC
```

## 孤立ノードの検出

```cypher
-- 接続のないノードを検出
MATCH (n:Person)
WHERE NOT (n)-[:KNOWS]-()
RETURN n.name, "isolated" AS status

-- または連結成分を使用
CALL db.connectedComponents("KNOWS")
YIELD component_id, nodes, node_count
WHERE node_count = 1
RETURN [n IN nodes | n.name][0] AS isolated_node
```

## グラフの連結性の確認

```cypher
-- グラフ全体が連結しているか確認
CALL db.connectedComponents("ROAD")
YIELD component_id
RETURN count(DISTINCT component_id) AS num_components
-- 1 であれば完全連結
```

## ラベル伝播法（コミュニティ検出）

連結成分よりも細かいコミュニティ構造を検出するには、ラベル伝播法を使用します。

```cypher
-- ラベル伝播でコミュニティを検出
CALL db.labelPropagation("KNOWS", 50)  -- 50 回反復
YIELD node, community_id
RETURN community_id, collect(node.name) AS members, count(*) AS size
ORDER BY size DESC
```

ラベル伝播法は連結成分と異なり、密に接続したサブグラフ（コミュニティ）を検出します。ソーシャルグラフでの友人グループの発見に適しています。
