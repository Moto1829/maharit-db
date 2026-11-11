---
title: サイクル検出
parent: グラフアルゴリズム
nav_order: 5
---

# サイクル検出

MaharitDB はグラフ内のサイクル（閉路）を検出するための 3 つの関数を提供しています。依存関係グラフの検証や有向非巡回グラフ（DAG）の確認、トポロジカル順序の取得などに活用できます。`maharit-core` の Rust API で、リレーションシップの向きに沿って判定します。

## 概要

| 関数 | 目的 | 計算量 |
|------|------|--------|
| `has_cycle()` | サイクルの有無を判定 | O(V + E) |
| `find_cycles()` | すべてのサイクルを列挙 | O(V + E) |
| `topological_sort()` | トポロジカルソートを取得 | O(V + E) |

主なユースケース：

- **依存関係の検証**: パッケージ管理システムで循環依存を検出する
- **タスクスケジューリング**: 実行順序に矛盾がないことを確認する
- **データパイプライン**: 処理フローに循環がないことを保証する
- **スキーマ設計**: 外部キー参照に循環がないことを確認する

## has_cycle()

グラフにサイクルが含まれているかどうかを DFS（深さ優先探索）で判定します。再帰スタック（`rec_stack`）を用いたバックエッジ検出により、最初のサイクルを発見した時点で `true` を返します。

```rust
pub fn has_cycle(graph: &Graph) -> bool
```

### Rust での使用例

```rust
use maharit_core::{Graph, NodeId, algorithms};

fn node(graph: &mut Graph, label: &str, name: &str) -> NodeId {
    let id = graph.create_node(label);
    graph.get_node_mut(id).unwrap().set_property("name", name);
    id
}

fn main() {
    let mut graph = Graph::new();

    let a = node(&mut graph, "Task", "A");
    let b = node(&mut graph, "Task", "B");
    let c = node(&mut graph, "Task", "C");

    // A -> B -> C（サイクルなし）
    graph.create_edge(a, b, "DEPENDS_ON").unwrap();
    graph.create_edge(b, c, "DEPENDS_ON").unwrap();

    println!("DAG: {}", !algorithms::has_cycle(&graph)); // DAG: true

    // C -> A を追加してサイクルを作る
    graph.create_edge(c, a, "DEPENDS_ON").unwrap();

    println!("DAG: {}", !algorithms::has_cycle(&graph)); // DAG: false
}
```

サイクルが存在しない場合は `false`、1 つでも存在する場合は `true` を返します。サイクルの有無だけ確認できればよい場合は、`find_cycles()` よりも高速です。

## find_cycles()

グラフ内のすべてのサイクルを列挙します。DFS のバックエッジ検出を用いており、サイクルを構成するノード ID のリストを返します。

```rust
pub fn find_cycles(graph: &Graph) -> Vec<Vec<NodeId>>
```

### 返り値の形式

`Vec<Vec<NodeId>>` を返します。各内部ベクタはサイクルを構成するノード ID のリストです。

例: `A -> B -> C -> A` というサイクルの場合、`[NodeId_A, NodeId_B, NodeId_C]` のようなベクタが返されます（終点と始点の重複はありません）。

サイクルが存在しない場合は空の `Vec` を返します。

### Rust での使用例

```rust
use maharit_core::{Graph, NodeId, PropertyValue, algorithms};

fn node(graph: &mut Graph, label: &str, name: &str) -> NodeId {
    let id = graph.create_node(label);
    graph.get_node_mut(id).unwrap().set_property("name", name);
    id
}

fn name(graph: &Graph, id: NodeId) -> String {
    match graph.get_node(id).and_then(|n| n.get_property("name")) {
        Some(PropertyValue::String(s)) => s.clone(),
        _ => id.to_string(),
    }
}

fn main() {
    let mut graph = Graph::new();

    let pkg_a = node(&mut graph, "Package", "pkg-a");
    let pkg_b = node(&mut graph, "Package", "pkg-b");
    let pkg_c = node(&mut graph, "Package", "pkg-c");
    let pkg_d = node(&mut graph, "Package", "pkg-d");

    // pkg-a -> pkg-b -> pkg-c -> pkg-b（循環依存）
    graph.create_edge(pkg_a, pkg_b, "REQUIRES").unwrap();
    graph.create_edge(pkg_b, pkg_c, "REQUIRES").unwrap();
    graph.create_edge(pkg_c, pkg_b, "REQUIRES").unwrap();
    // pkg-a -> pkg-d（正常な依存）
    graph.create_edge(pkg_a, pkg_d, "REQUIRES").unwrap();

    let cycles = algorithms::find_cycles(&graph);

    if cycles.is_empty() {
        println!("循環依存はありません");
    } else {
        println!("循環依存が {} 件見つかりました:", cycles.len());
        for (i, cycle) in cycles.iter().enumerate() {
            let names: Vec<String> = cycle
                .iter()
                .map(|&node_id| {
                    name(&graph, node_id)
                })
                .collect();
            println!("  サイクル {}: {}", i + 1, names.join(" -> "));
        }
    }
}
```

出力例：

```
循環依存が 1 件見つかりました:
  サイクル 1: pkg-b -> pkg-c
```

## topological_sort()

Kahn のアルゴリズム（入次数ベースの BFS）を用いてトポロジカルソートを行います。グラフが DAG（有向非巡回グラフ）である場合のみ意味のある順序を返します。

```rust
pub fn topological_sort(graph: &Graph) -> Option<Vec<NodeId>>
```

### 返り値の意味

- `Some(Vec<NodeId>)`: グラフにサイクルがなく、トポロジカル順序でノード ID を並べたリストを返します。各リレーションシップの始点が終点より先に来る順序です。
- `None`: グラフにサイクルが存在するため、トポロジカル順序を定義できません。

### Rust での使用例

```rust
use maharit_core::{Graph, NodeId, PropertyValue, algorithms};

fn node(graph: &mut Graph, label: &str, name: &str) -> NodeId {
    let id = graph.create_node(label);
    graph.get_node_mut(id).unwrap().set_property("name", name);
    id
}

fn name(graph: &Graph, id: NodeId) -> String {
    match graph.get_node(id).and_then(|n| n.get_property("name")) {
        Some(PropertyValue::String(s)) => s.clone(),
        _ => id.to_string(),
    }
}

fn main() {
    let mut graph = Graph::new();

    // ビルドタスクの依存グラフ
    let install_deps = node(&mut graph, "Task", "install_deps");
    let compile     = node(&mut graph, "Task", "compile");
    let test        = node(&mut graph, "Task", "test");
    let package     = node(&mut graph, "Task", "package");
    let deploy      = node(&mut graph, "Task", "deploy");

    graph.create_edge(install_deps, compile, "BEFORE").unwrap();
    graph.create_edge(compile, test, "BEFORE").unwrap();
    graph.create_edge(test, package, "BEFORE").unwrap();
    graph.create_edge(package, deploy, "BEFORE").unwrap();

    match algorithms::topological_sort(&graph) {
        Some(order) => {
            let names: Vec<String> = order
                .iter()
                .map(|&node_id| {
                    name(&graph, node_id)
                })
                .collect();
            println!("実行順序: {}", names.join(" -> "));
        }
        None => {
            eprintln!("エラー: タスク依存グラフにサイクルが存在します");
        }
    }
}
```

出力例：

```
実行順序: install_deps -> compile -> test -> package -> deploy
```

サイクルが存在する場合：

```rust
// サイクルを追加
graph.create_edge(deploy, compile, "BEFORE").unwrap();

match algorithms::topological_sort(&graph) {
    Some(_) => println!("DAGです"),
    None    => println!("サイクルが検出されました"), // こちらが出力される
}
```

## ユースケース例

### パッケージ依存解決

```rust
use maharit_core::{Graph, NodeId, algorithms};

fn resolve_install_order(graph: &Graph) -> Result<Vec<NodeId>, String> {
    algorithms::topological_sort(graph)
        .ok_or_else(|| "循環依存が存在します。インストール順序を決定できません。".to_string())
}
```

### タスク依存グラフの検証

```rust
use maharit_core::{Graph, NodeId, algorithms};

fn validate_pipeline(graph: &Graph) -> Result<(), Vec<Vec<NodeId>>> {
    let cycles = algorithms::find_cycles(graph);
    if cycles.is_empty() {
        Ok(())
    } else {
        Err(cycles)
    }
}
```

### CI/CD パイプラインの整合性チェック

```rust
use maharit_core::{Graph, algorithms};

fn check_pipeline_integrity(graph: &Graph) -> bool {
    // サイクルがなく、かつトポロジカル順序が存在することを確認
    !algorithms::has_cycle(graph)
}
```

## パフォーマンス

すべての関数は **O(V + E)** の時間計算量で動作します（V はノード数、E はエッジ数）。

| 関数 | 時間計算量 | 空間計算量 | 備考 |
|------|-----------|-----------|------|
| `has_cycle()` | O(V + E) | O(V) | 最初のサイクル発見で早期終了 |
| `find_cycles()` | O(V + E) | O(V) | 全サイクルを列挙するため早期終了なし |
| `topological_sort()` | O(V + E) | O(V) | Kahn のアルゴリズム（入次数BFS） |

サイクルの有無だけを確認したい場合は `has_cycle()` が最も効率的です。全サイクルの列挙が必要な場合は `find_cycles()` を使用してください。

`NodeId` は `u64` 型エイリアスです。大規模グラフ（数百万ノード規模）でも線形時間で処理できますが、`find_cycles()` は多数のサイクルが存在する場合に返り値のメモリ使用量が増大する点に注意してください。
