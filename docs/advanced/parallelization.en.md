---
title: Parallelism and performance
parent: Advanced topics
nav_order: 6
source_commit: f427cc91
---

# Parallelism and performance

MaharitDB's graph algorithms switch to parallel processing automatically on large graphs. Parallelism uses [Rayon](https://docs.rs/rayon), whose data-parallel iterators cut the time spent on graph analysis.

## How automatic parallelism works

Parallelism for graph algorithms is implemented in `algorithms.rs` in `maharit-core`. An algorithm checks the graph's node count and uses Rayon's parallel iterators (`par_iter()`) at or above a threshold; below it, it runs sequentially.

```rust
// From algorithms.rs
const PARALLEL_THRESHOLD: usize = 500;
```

### Why there is a threshold

Rayon provides parallelism with a thread pool, but distributing tasks to the pool and synchronizing them has overhead. On graphs with few nodes, that overhead can outweigh the computation and make parallel runs slower than sequential ones.

The 500-node threshold balances both cases:

- Small graphs (< 500 nodes): avoid the thread-pool overhead and finish quickly sequentially
- Large graphs (>= 500 nodes): make full use of multi-core CPUs in parallel

## Parallelism by algorithm

| Algorithm | Parallel | What runs in parallel | Notes |
|---|---|---|---|
| Closeness Centrality | Yes | The BFS from each source node | `par_iter()` at >= 500 nodes |
| Betweenness Centrality | Yes | The outer loop of Brandes' algorithm | `par_iter()` at >= 500 nodes |
| PageRank | Partly | The sum of dangling-node scores | Only the dangling sum is parallel at >= 500 nodes |
| Degree Centrality | No | — | Sequential (edge scan only) |
| Connected Components | No | — | Sequential BFS (has dependencies) |
| Strongly Connected Components | No | — | Kosaraju's algorithm (sequential DFS) |
| Label Propagation | No | — | Sequential because iterations depend on each other |
| Cycle Detection | No | — | Sequential DFS |
| Topological Sort | No | — | Kahn's algorithm (sequential) |
| Shortest Path (Dijkstra) | No | — | Sequential priority queue |

### Details

**Closeness Centrality**: the BFS from each node is independent, so all nodes are processed in parallel.

```rust
if n >= PARALLEL_THRESHOLD {
    node_ids.par_iter().map(compute_one).collect()
} else {
    node_ids.iter().map(compute_one).collect()
}
```

**Betweenness Centrality** parallelizes the outer loop of Brandes' algorithm (the partial centrality per source node). Each thread builds a local map of partial centralities, and the maps are merged at the end. Only shared immutable references to `Graph` (`&Graph`) are used, so the parallel run is safe.

```rust
let partial_maps: Vec<HashMap<NodeId, f64>> = if n >= PARALLEL_THRESHOLD {
    nodes
        .par_iter()
        .map(|&source| brandes_single(graph, &nodes, source))
        .collect()
} else {
    nodes
        .iter()
        .map(|&source| brandes_single(graph, &nodes, source))
        .collect()
};
```

**PageRank** accumulates edge contributions by writing to a `HashMap`, which must run sequentially, so only the sum of dangling-node scores (nodes with out-degree 0) is parallelized.

```rust
let dangling_sum: f64 = if use_parallel {
    dangling_nodes.par_iter().map(|id| scores[id]).sum()
} else {
    dangling_nodes.iter().map(|id| scores[id]).sum()
};
```

**Label Propagation** is not parallelized, because each iteration's updates depend on the previous iteration's results.

## Controlling the number of threads

By default Rayon uses a global thread pool with as many threads as logical CPUs. Control the thread count with an environment variable.

```bash
# Limit to 4 threads
RAYON_NUM_THREADS=4 maharit server
```

To configure Rayon's global thread pool from code:

```rust
rayon::ThreadPoolBuilder::new()
    .num_threads(4)
    .build_global()
    .unwrap();
```

The global thread pool can be configured only once; calling this again returns an error.

## Performance characteristics

### Small graphs (< 500 nodes)

Sequential processing is used. Without the overhead of distributing tasks to Rayon's thread pool, memory access is efficient for almost every algorithm.

### Large graphs (>= 500 nodes)

Rayon's parallel iterators are enabled. Throughput grows with the number of cores, with these caveats:

- **Closeness Centrality**: expect close to linear scaling (each BFS is independent)
- **Betweenness Centrality**: the merge of partial results is an O(N) serial step
- **PageRank**: only part of the dangling-sum computation is parallel, so the benefit is limited

### Memory use

The parallel Betweenness Centrality keeps a local `HashMap<NodeId, f64>` per thread, so it uses additional memory proportional to the number of threads compared with the sequential version. Keep this in mind when using all cores on a large graph.
