---
title: All paths
parent: Graph algorithms
nav_order: 2
source_commit: 52f38528
---

# All-paths search

Lists every path between two nodes, up to a maximum number of hops. In Cypher, use a variable-length pattern; in the Rust API, use `all_paths`. Both return only paths that do not visit the same node twice (simple paths).

## Cypher: variable-length patterns

```cypher
// Every path from Alice to Bob within 4 hops
MATCH p = (a:Person {name: "Alice"})-[:KNOWS*1..4]->(b:Person {name: "Bob"})
RETURN [n IN nodes(p) | n.name] AS names, length(p) AS hops
ORDER BY hops ASC
```

```cypher
// Number of paths per hop count
MATCH p = (a:Person {name: "Alice"})-[:KNOWS*1..4]->(b:Person {name: "Bob"})
RETURN length(p) AS hops, count(*) AS path_count
ORDER BY hops
```

```cypher
// Any relationship type
MATCH p = (a:Node {id: 1})-[*1..3]->(b:Node {id: 10})
RETURN p, length(p)
```

```cypher
// Routes within 5 hops by road or ferry (types separated by `|` match any of them)
MATCH p = (a:City {name: "Tokyo"})-[:ROAD|FERRY*1..5]->(b:City {name: "Sapporo"})
RETURN [n IN nodes(p) | n.name] AS route, length(p) AS hops
ORDER BY hops
```

## Rust API: all_paths

`maharit_core::all_paths(&graph, from, to, max_depth)` lists paths with depth-first search (DFS) and backtracking. It follows relationship direction and does not distinguish types. `max_depth` of `None` means no limit.

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

Example output (the order of the paths can vary between runs):

```text
Found 3 paths:
  Alice -> Bob -> Dave (hops: 2)
  Alice -> Charlie -> Bob -> Dave (hops: 3)
  Alice -> Charlie -> Dave (hops: 2)
```

## A note on cycles

Both variable-length patterns and `all_paths` return only simple paths (paths that do not visit the same node twice). Paths that return to the start node are not included.

```cypher
// Simple paths only (the default behavior)
MATCH p = (a:Person {name: "Alice"})-[:KNOWS*1..5]->(b:Person {name: "Alice"})
RETURN p  // No result (a simple path cannot return to Alice)
```

## Path statistics

```cypher
// Aggregate statistics over all paths
MATCH p = (a:Person {name: "Alice"})-[:KNOWS*1..4]->(b:Person)
WHERE b.name <> "Alice"
RETURN
  b.name AS target,
  count(p) AS path_count,
  min(length(p)) AS min_hops,
  max(length(p)) AS max_hops
ORDER BY min_hops, path_count DESC
```

## Performance notes

All-paths search is prone to combinatorial explosion.

- Always set an upper bound on the number of hops
- On dense graphs, the number of paths grows exponentially
- When there are many paths, limit them with `LIMIT`

```cypher
// Get only the first 100 paths
MATCH p = (a:Person {name: "Alice"})-[:KNOWS*1..4]->(b:Person)
RETURN p
LIMIT 100
```

## Use cases

- **Reachability in social graphs**: through how many people can two people meet
- **Network redundancy**: finding single points of failure
- **Listing every dependency route**: software package dependency trees
- **Logistics routes**: listing every possible delivery route
