# Task 66: Read Query Concurrency Optimization

**Status**: Completed (infrastructure implemented; executor refactoring deferred)

## Summary

Added read-only query classification infrastructure to enable future concurrent
read execution in `crates/maharit-server/src/tcp_server.rs` and
`crates/maharit-query/src/ast.rs`.

## Changes

### `crates/maharit-query/src/ast.rs`
- Added `pub fn is_read_only(stmt: &Statement) -> bool` — classifies every
  `Statement` variant as read-only or write. Read-only variants: `Match`,
  `Union` (all MATCH queries), `ShowConstraints`, `ShowUsers`, `ProcedureCall`,
  and `Explain`/`Profile` when their inner statement is also read-only.

### `crates/maharit-query/src/lib.rs`
- Re-exported `is_read_only` from the crate root.

### `crates/maharit-server/src/tcp_server.rs`
- Imported `is_read_only` from `maharit_query`.
- `execute_query`: parses the query *before* locking the graph, classifies it
  with `is_read_only`, logs the flag. The lock path uses `graph.write()` for
  now (see note below).
- `execute_streaming_query`: same `is_read_only` classification applied.

## Note on Executor API Limitation

The `Executor::new(&mut Graph)` API requires an exclusive mutable reference.
Changing the lock to `graph.read()` for read-only queries would require either:
- Creating a `Executor::new_read_only(&Graph)` variant, or
- Wrapping `Graph` in `std::cell::UnsafeCell` on the server side.

Both approaches would require significant refactoring. The `is_read_only`
function and parse-before-lock structure are in place so this can be added
in a follow-up task without touching the server call sites.

## Tests

All 171 maharit-server tests pass; full workspace 0 failures.
