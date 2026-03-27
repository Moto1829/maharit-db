---
title: ASCII テキスト可視化
parent: 可視化
nav_order: 2
---

# ASCII テキスト可視化

`maharit-viz` クレートの `AsciiRenderer` は、グラフをターミナルで読めるテキスト形式に変換します。外部ツール不要で、デバッグやログ出力に適しています。

ボックス描画文字（`├` `└` `│` `─`）を使ったツリー表示により、グラフ構造を直感的に確認できます。

## 3 つのメソッド

| メソッド | 探索方式 | 用途 |
|---------|---------|------|
| `render` | 全ノード列挙 | グラフ全体の一覧確認 |
| `render_tree` | DFS（深さ優先） | 木構造・階層関係の確認 |
| `render_layers` | BFS（幅優先） | 距離・到達レイヤーの確認 |

## render — グラフ全体の表示

```rust
use maharit_viz::ascii::AsciiRenderer;
use maharit_core::Graph;

fn main() {
    let mut graph = Graph::new();
    let alice = graph.create_node("Person");
    let bob   = graph.create_node("Person");
    let acme  = graph.create_node("Company");

    graph.create_edge(alice, bob,  "KNOWS").unwrap();
    graph.create_edge(alice, acme, "WORKS_AT").unwrap();
    graph.create_edge(bob,   acme, "WORKS_AT").unwrap();

    let output = AsciiRenderer::render(&graph);
    print!("{}", output);
}
```

出力例：

```
Graph: 3 nodes, 3 edges

(0):Person
 ├──[:KNOWS]──> (1)
 └──[:WORKS_AT]──> (2)

(1):Person
 └──[:WORKS_AT]──> (2)

(2):Company
```

グラフが空の場合は `(empty graph)` を返します。

## render_tree — DFS ツリービュー

指定したルートノードから DFS で到達可能なノードを木構造で表示します。
`max_depth` でどこまで展開するかを制御できます。

```rust
use maharit_viz::ascii::AsciiRenderer;
use maharit_core::Graph;

fn main() {
    let mut graph = Graph::new();
    let root  = graph.create_node("Root");
    let child1 = graph.create_node("Child");
    let child2 = graph.create_node("Child");
    let leaf   = graph.create_node("Leaf");

    graph.create_edge(root,   child1, "HAS").unwrap();
    graph.create_edge(root,   child2, "HAS").unwrap();
    graph.create_edge(child1, leaf,   "HAS").unwrap();

    // ルートノード ID と最大深度を指定
    let output = AsciiRenderer::render_tree(&graph, root, 5);
    print!("{}", output);
}
```

出力例：

```
(0):Root
├─[:HAS]─>
│   (1):Child
│   └─[:HAS]─>
│       (3):Leaf
└─[:HAS]─>
    (2):Child
```

### 循環参照の検出

`render_tree` は訪問済みノードを `HashSet` で管理しており、循環が検出された時点でそのノードに `[circular]` を付与して展開を停止します。無限ループにはなりません。

```rust
let mut graph = Graph::new();
let a = graph.create_node("A");
let b = graph.create_node("B");

graph.create_edge(a, b, "NEXT").unwrap();
graph.create_edge(b, a, "BACK").unwrap();  // 循環

let output = AsciiRenderer::render_tree(&graph, a, 10);
print!("{}", output);
```

出力例：

```
(0):A
└─[:NEXT]─>
    (1):B
    └─[:BACK]─>
        (0):A [circular]
```

## render_layers — BFS レイヤービュー

指定したルートノードから BFS で探索し、ルートからの距離（ホップ数）ごとにレイヤーを分けて表示します。

```rust
use maharit_viz::ascii::AsciiRenderer;
use maharit_core::Graph;

fn main() {
    let mut graph = Graph::new();
    let root  = graph.create_node("Root");
    let a     = graph.create_node("A");
    let b     = graph.create_node("B");
    let leaf  = graph.create_node("Leaf");

    graph.create_edge(root, a,    "TO").unwrap();
    graph.create_edge(root, b,    "TO").unwrap();
    graph.create_edge(a,    leaf, "TO").unwrap();

    let output = AsciiRenderer::render_layers(&graph, root, 3);
    print!("{}", output);
}
```

出力例：

```
Layer 0 (root):
  (0):Root

Layer 1:
  (1):A
  (2):B

Layer 2:
  (3):Leaf
```

`max_depth` を超えた深さのノードは表示されません。また、一度訪問したノードは重複して表示されません。

## ボックス描画文字について

`AsciiRenderer` はボックス描画文字（Box-drawing characters）を使用しており、Unicode 対応ターミナルで正しく表示されます。

| 文字 | 意味 |
|-----|------|
| `├` | 中間ブランチ（後続の兄弟あり） |
| `└` | 末端ブランチ（最後の兄弟） |
| `│` | 縦方向の継続線 |
| `─` | 横方向の接続線 |

Windows の旧来のコンソール（CP932）では文字化けする場合があります。その場合は UTF-8 コードページ（`chcp 65001`）に切り替えてください。
