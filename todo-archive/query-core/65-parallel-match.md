# Task 65: MATCH候補フィルタリングの並列化

## 概要

Parallelized the node candidate scan in `match_node_pattern()` in
`crates/maharit-query/src/executor.rs`.

## 実装内容

- Added `rayon = "1.10"` to `crates/maharit-query/Cargo.toml`
- Added `use rayon::prelude::*` and `PARALLEL_MATCH_THRESHOLD = 500` to
  `executor.rs`
- `match_node_pattern()`: when candidate node count >= threshold, phase 1
  filters node IDs with `par_iter()` (parallel, read-only), phase 2 assembles
  bindings sequentially. Below threshold uses the existing sequential path.

## 設計メモ

- `node_matches_pattern(&self, ...)` only performs reads through `&Graph`
  and `evaluate_expression(&self, ...)` — no mutation occurs, making parallel
  execution safe.
- Pattern closure captures `&self` (immutable borrow) so rayon can share it
  across threads.

## テスト

All 424 maharit-query tests pass.
