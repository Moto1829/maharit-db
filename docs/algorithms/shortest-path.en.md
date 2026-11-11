---
title: Shortest paths
parent: Graph algorithms
nav_order: 1
source_commit: 52f38528
---

# Shortest paths

Find paths with the fewest hops with Cypher's `shortestPath` / `allShortestPaths`, and shortest paths weighted by a relationship property with the Rust API (`Dijkstra` / `AStar` in `maharit-core`).

## Cypher: shortestPath / allShortestPaths

Breadth-first search (BFS) returns the path with the fewest hops.

```cypher
// One shortest path
MATCH p = shortestPath((a:Person {name: "Alice"})-[:KNOWS*]-(b:Person {name: "Charlie"}))
RETURN [n IN nodes(p) | n.name] AS names, length(p) AS hops
```

```cypher
// Every path with the fewest hops
MATCH p = allShortestPaths((a:Person {name: "Alice"})-[:KNOWS*]-(b:Person {name: "Charlie"}))
RETURN [n IN nodes(p) | n.name] AS names
```

```cypher
// Follow a direction, up to 5 hops
MATCH p = shortestPath((a:City {name: "Tokyo"})-[:ROAD*..5]->(b:City {name: "Osaka"}))
RETURN [c IN nodes(p) | c.name] AS cities, length(p) AS hops
```

- `-[:KNOWS*]-` ignores direction; `-[:ROAD*]->` follows the arrow only
- The minimum number of hops must be 0 or 1 (lower bounds such as `*2..` are not allowed)
- If there is no path, no row is returned

## Rust API: weighted shortest paths (Dijkstra)

Shortest paths weighted by a relationship property (such as `distance`) cannot be computed in Cypher. Use `Dijkstra` from the Rust API. Weights must be 0 or more (negative weights are not supported).

```rust
use maharit_core::{Dijkstra, Graph, NodeId};

fn city(graph: &mut Graph, name: &str) -> NodeId {
    let id = graph.create_node("City");
    graph.get_node_mut(id).unwrap().set_property("name", name);
    id
}

fn road(graph: &mut Graph, from: NodeId, to: NodeId, km: f64) {
    let id = graph.create_edge(from, to, "ROAD").unwrap();
    graph.get_edge_mut(id).unwrap().set_property("distance", km);
}

fn main() {
    let mut graph = Graph::new();
    let tokyo = city(&mut graph, "Tokyo");
    let nagoya = city(&mut graph, "Nagoya");
    let osaka = city(&mut graph, "Osaka");
    road(&mut graph, tokyo, nagoya, 350.0);
    road(&mut graph, nagoya, osaka, 190.0);
    road(&mut graph, tokyo, osaka, 600.0);

    // Fewest hops (BFS): Tokyo → Osaka
    let path = maharit_core::shortest_path(&graph, tokyo, osaka).unwrap();
    println!("hops: {}", path.len() - 1); // 1

    // Smallest total distance: Tokyo → Nagoya → Osaka (540)
    let dijkstra = Dijkstra::with_weight_property(&graph, "distance");
    let best = dijkstra.find_path(tokyo, osaka).unwrap();
    println!("nodes: {:?}, total: {}", best.nodes, best.total_weight);

    // Distances from one source to every node
    let distances = dijkstra.distances_from(tokyo);
    println!("{:?}", distances.get(&osaka)); // Some(540.0)
}
```

- `Dijkstra::new(&graph)` treats every weight as 1. Relationships without the weight property also count as 1
- `Dijkstra::with_weight_fn(&graph, |edge| …)` takes a function that computes the weight
- If you have an estimate of the distance to the target (a heuristic), `AStar::new(&graph, |from, to| …)` explores fewer nodes
- The resulting `WeightedPath` has `nodes` (node IDs), `edges` (relationship IDs), and `total_weight`

## Performance

- Unweighted shortest paths use BFS, in time proportional to the relationships traversed
- An upper bound on the hops of a variable-length pattern (`*..N`) limits the search
- `allShortestPaths` can return large results on graphs with many shortest paths
