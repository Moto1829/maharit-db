---
title: ASCII visualization
parent: Visualization
nav_order: 2
source_commit: f427cc91
---

# ASCII visualization

`AsciiRenderer` in the `maharit-viz` crate turns a graph into text you can read in a terminal. It needs no external tools and suits debugging and log output.

A tree view drawn with box-drawing characters (`├` `└` `│` `─`) makes the graph structure easy to follow.

## Three methods

| Method | Traversal | Use |
|---------|---------|------|
| `render` | Lists every node | Overview of the whole graph |
| `render_tree` | DFS (depth-first) | Trees and hierarchies |
| `render_layers` | BFS (breadth-first) | Distances and reachability layers |

## render — the whole graph

```rust
use maharit_viz::AsciiRenderer;
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

Example output:

```
Graph: 3 nodes, 3 edges

(0):Person
 ├──[:KNOWS]──> (1)
 └──[:WORKS_AT]──> (2)

(1):Person
 └──[:WORKS_AT]──> (2)

(2):Company
```

An empty graph returns `(empty graph)`.

## render_tree — DFS tree view

Shows the nodes reachable from a root node by DFS as a tree.
`max_depth` controls how deep it expands.

```rust
use maharit_viz::AsciiRenderer;
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

    // Give the root node ID and the maximum depth
    let output = AsciiRenderer::render_tree(&graph, root, 5);
    print!("{}", output);
}
```

Example output:

```
(0):Root
├─[:HAS]─>
│   (1):Child
│   └─[:HAS]─>
│       (3):Leaf
└─[:HAS]─>
    (2):Child
```

### Detecting cycles

`render_tree` tracks visited nodes in a `HashSet`. When it finds a cycle, it marks the node with `[circular]` and stops expanding there, so it never loops forever.

```rust
let mut graph = Graph::new();
let a = graph.create_node("A");
let b = graph.create_node("B");

graph.create_edge(a, b, "NEXT").unwrap();
graph.create_edge(b, a, "BACK").unwrap();  // Cycle

let output = AsciiRenderer::render_tree(&graph, a, 10);
print!("{}", output);
```

Example output:

```
(0):A
└─[:NEXT]─>
    (1):B
    └─[:BACK]─>
        (0):A [circular]
```

## render_layers — BFS layer view

Searches from a root node by BFS and shows the nodes in layers by distance (hops) from the root.

```rust
use maharit_viz::AsciiRenderer;
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

Example output:

```
Layer 0 (root):
  (0):Root

Layer 1:
  (1):A
  (2):B

Layer 2:
  (3):Leaf
```

Nodes deeper than `max_depth` are not shown, and a node is shown only once even if it is reached again.

## Box-drawing characters

`AsciiRenderer` uses box-drawing characters, which display correctly in Unicode-capable terminals.

| Character | Meaning |
|-----|------|
| `├` | Middle branch (more siblings follow) |
| `└` | Last branch (the last sibling) |
| `│` | Vertical continuation |
| `─` | Horizontal connector |

The legacy Windows console (CP932) may garble them. In that case, switch to the UTF-8 code page (`chcp 65001`).
