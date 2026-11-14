---
title: Connected components
parent: Graph algorithms
nav_order: 4
source_commit: 52f38528
---

# Connected components

Splits a graph into groups of nodes that are connected to each other. Computed with Cypher procedures or with the Rust API of the `maharit-core` crate.

## Cypher: procedures

| Procedure | YIELD | Configuration (map) |
|---|---|---|
| `db.connectedComponents` | `node`, `componentId` | — |
| `db.stronglyConnectedComponents` | `node`, `componentId` | — |
| `db.labelPropagation` | `node`, `communityId` | `maxIterations` (default 10) |

The configuration map is optional. Every procedure accepts `nodeLabels` (only nodes with any of these labels) and `relationshipTypes` (follow only relationships of these types); without them the whole graph is used. Rows come back in node-ID order. `componentId` numbers components 0, 1, 2, … in order of the smallest node ID they contain. `communityId` is the ID of the community's representative node.

```cypher
// Number of nodes per component
CALL db.connectedComponents() YIELD node, componentId
RETURN componentId, count(node) AS size
ORDER BY size DESC
```

```cypher
// Write the component number back as a property
CALL db.connectedComponents({relationshipTypes: ["KNOWS"]}) YIELD node, componentId
SET node.component = componentId
```

## Connected components (weak)

Ignores relationship direction and groups connected nodes into one component.

```rust
use maharit_core::{Graph, connected_components};

fn main() {
    let mut graph = Graph::new();
    let n: Vec<_> = (0..5).map(|_| graph.create_node("Person")).collect();
    graph.create_edge(n[0], n[1], "KNOWS").unwrap();
    graph.create_edge(n[2], n[1], "KNOWS").unwrap(); // Direction does not matter
    graph.create_edge(n[3], n[4], "KNOWS").unwrap();

    let components = connected_components(&graph);
    println!("{} components", components.len()); // 2
    for (i, component) in components.iter().enumerate() {
        println!("component {i}: {} nodes", component.len());
    }
}
```

The result is a `Vec<HashSet<NodeId>>`, one set of node IDs per component. A node with no relationships forms a component of its own.

Typical uses:

- One component means the whole graph is connected
- Components of size 1 are isolated nodes
- The distribution of component sizes reveals skewed data or missing connections

## Strongly connected components

Groups of nodes that can reach each other following relationship direction (Kosaraju's algorithm). For example, A → B → C → A is one strongly connected component, but with only A → B, A and B are separate components.

```rust
let scc = maharit_core::strongly_connected_components(&graph);
// Components with two or more nodes contain a cycle (mutual references)
let cyclic: Vec<_> = scc.iter().filter(|c| c.len() > 1).collect();
```

Typical uses: groups that follow each other, dependencies that reference each other in a cycle.

## Label propagation (community detection)

Each node repeatedly adopts the most common community ID among its neighbors (in either direction), finding densely connected groups (communities).

```rust
let communities = maharit_core::label_propagation(&graph, 50); // Up to 50 iterations
// communities: HashMap<NodeId, u64> (node → community ID)
```

- Each node starts with its own ID as its community ID
- Ties go to the smaller ID, so results are the same on every run
- Stops when nothing changes or the iteration limit is reached

## Writing results back to the graph

With the Cypher procedures, write the results back with `SET node.component = componentId` ([above](#cypher-procedures)). When you compute with the Rust API, write them back as properties like this.

```rust
for (i, component) in maharit_core::connected_components(&graph).iter().enumerate() {
    for &id in component {
        graph.get_node_mut(id).unwrap().set_property("component", i as i64);
    }
}
```

```cypher
// After writing back: number of nodes per component
MATCH (n)
RETURN n.component AS component, count(n) AS size
ORDER BY size DESC
```
