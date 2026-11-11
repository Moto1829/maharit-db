---
title: PageRank and centrality
parent: Graph algorithms
nav_order: 3
source_commit: 52f38528
---

# PageRank and centrality

Measures of how important a node is. They are computed over a whole `Graph` with the Rust API of the `maharit-core` crate.

!!! note "Not available from Cypher"
    Centrality is computed with the Rust API only; there are no Cypher procedures (`CALL …`) for it. Every measure ignores relationship types and covers the whole graph.

## PageRank

Nodes that receive many links, or links from important nodes, score higher. Relationship direction is used (`(a)-->(b)` is a link from a to b).

```rust
use maharit_core::{Graph, PageRankConfig, pagerank};

fn main() {
    let mut graph = Graph::new();
    let pages: Vec<_> = (0..4).map(|_| graph.create_node("Page")).collect();
    for (from, to) in [(0, 1), (0, 2), (1, 2), (2, 0), (3, 2)] {
        graph.create_edge(pages[from], pages[to], "LINKS").unwrap();
    }

    let result = pagerank(&graph, &PageRankConfig::default());
    println!("iterations: {}, converged: {}", result.iterations, result.converged);

    let mut ranked: Vec<_> = result.scores.iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(a.1));
    for (node, score) in ranked {
        println!("{node}: {score:.4}");
    }
}
```

`PageRankConfig` fields:

| Field | Default | Description |
|---|---|---|
| `damping` | `0.85` | Damping factor: the probability of continuing to follow links |
| `max_iterations` | `100` | Maximum number of iterations |
| `tolerance` | `1e-6` | Considered converged when scores change less than this |

The score of nodes without outgoing links (dangling nodes) is spread evenly over all nodes.

## Betweenness centrality

How often a node lies on the shortest paths between other nodes. Nodes with high values are "bridges" between groups. Computed with Brandes' algorithm, counting shortest paths that follow relationship direction.

```rust
let scores = maharit_core::betweenness_centrality(&graph);
```

## Closeness centrality

Higher when the distances (in hops) from the node to other nodes are shorter. With `r` the number of reachable nodes and `d` the sum of the distances to them, it is `(r - 1) / d`, following relationship direction. A node that reaches nothing scores 0.

```rust
let scores = maharit_core::closeness_centrality(&graph);
```

## Degree centrality

The number of relationships attached to a node.

```rust
use maharit_core::DegreeCentrality;

let degree = DegreeCentrality::compute(&graph);
let in_degree = &degree.in_degree;       // Incoming
let out_degree = &degree.out_degree;     // Outgoing
let total = &degree.total_degree;        // Total
let normalized = degree.normalized(&graph); // Total divided by 2(n-1)
```

## Comparison

| Measure | Complexity | Good for |
|------|--------|-----------------|
| PageRank | O(k × E) (k = iterations) | Ranking web pages, influence |
| Betweenness centrality | O(V × E) | Finding bridge nodes |
| Closeness centrality | O(V × (V + E)) | Finding nodes at the center |
| Degree centrality | O(E) | Popularity, activity |

On graphs with 500 or more nodes, betweenness and closeness centrality are computed in parallel (see [Parallelism and performance](../advanced/parallelization.md)).

## Writing results back to the graph

Results come back as `HashMap<NodeId, f64>`. To use them from Cypher, write them back as properties.

```rust
let scores = maharit_core::betweenness_centrality(&graph);
for (id, score) in scores {
    if let Some(node) = graph.get_node_mut(id) {
        node.set_property("betweenness", score);
    }
}
```

To compute over a server's data, stop the server, load the data file with `maharit_storage::PersistentStorage::load_with_schema`, compute and write back, and save with `save_with_schema` (`load` / `save` lose the index and constraint definitions). A remaining WAL (`<data>.wal`) is replayed at startup, so make sure it was emptied when the server stopped.
