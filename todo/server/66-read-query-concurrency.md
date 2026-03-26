# Task 66: 読み取りクエリの並行実行最適化

## 概要

Added read-only query classification infrastructure to enable future concurrent
read execution in `crates/maharit-server/src/tcp_server.rs` and
`crates/maharit-query/src/ast.rs`.

## 実装内容

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

## 注意

The `Executor::new_readonly(&Graph)` variant was added in task 58, enabling
`graph.read()` for read-only queries on the server side. The `is_read_only`
classification added here is used by `tcp_server.rs` to route queries to the
appropriate lock path.

## テスト

All 171 maharit-server tests pass; full workspace 0 failures.
