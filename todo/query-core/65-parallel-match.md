# Task 65: Parallel MATCH Candidate Filtering

**Status**: Completed

## Summary

Parallelized the node candidate scan in `match_node_pattern()` in
`crates/maharit-query/src/executor.rs`.

## Changes

- Added `rayon = "1.10"` to `crates/maharit-query/Cargo.toml`
- Added `use rayon::prelude::*` and `PARALLEL_MATCH_THRESHOLD = 500` to
  `executor.rs`
- `match_node_pattern()`: when candidate node count >= threshold, phase 1
  filters node IDs with `par_iter()` (parallel, read-only), phase 2 assembles
  bindings sequentially. Below threshold uses the existing sequential path.

## Design Notes

- `node_matches_pattern(&self, ...)` only performs reads through `&Graph`
  and `evaluate_expression(&self, ...)` — no mutation occurs, making parallel
  execution safe.
- Pattern closure captures `&self` (immutable borrow) so rayon can share it
  across threads.

## Tests

All 424 maharit-query tests pass.
