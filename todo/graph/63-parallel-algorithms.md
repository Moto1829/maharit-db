# Task 63: Graph Algorithm Parallelization

**Status**: Completed

## Summary

Parallelized graph algorithms in `crates/maharit-core/src/algorithms.rs` using `rayon`.

## Changes

- Added `rayon = "1.10"` to `crates/maharit-core/Cargo.toml`
- Added `PARALLEL_THRESHOLD = 500` constant — algorithms fall back to sequential for small graphs
- `closeness_centrality`: outer BFS loop over nodes parallelized via `par_iter()`
- `betweenness_centrality`: extracted inner Brandes per-source computation into a pure function, outer loop parallelized via `par_iter()`, results merged sequentially
- `pagerank`: dangling-node sum parallelized via `par_iter().sum()` (iteration loop is inherently sequential)

## Tests

All 14 algorithm tests pass.
