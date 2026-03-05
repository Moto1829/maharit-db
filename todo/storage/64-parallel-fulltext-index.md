# Task 64: Parallel Fulltext Index Building

**Status**: Completed

## Summary

Parallelized the tokenization phase of fulltext index construction in
`crates/maharit-core/src/fulltext.rs`.

## Changes

- Added `use rayon::prelude::*` to `fulltext.rs`
- Added `PARALLEL_BUILD_THRESHOLD = 200` constant
- Added `Tokenizer::tokenize_cached()` — uses a `thread_local!` lindera
  tokenizer per rayon worker thread to avoid repeated dictionary loads
- Added `FulltextIndex::build_index(&[(NodeId, &str, &str)])` — two-phase
  parallel build: phase 1 tokenizes in parallel (`par_iter()`), phase 2
  writes into the inverted index sequentially
- Added `FulltextManager::build_index_bulk()` — bulk-index multiple nodes
  across all matching indexes using the parallel build path

## Tests

All 44 fulltext tests pass (including Japanese tests).
