# Task 72: Aggregation Query Optimization

## Status: completed

## Overview

Two performance and correctness improvements to aggregation query execution.

## Changes

### 1. COUNT(node_var) Short-circuit Optimization

**File**: `crates/maharit-query/src/executor.rs` — `evaluate_aggregate`

In the `AggregateFunction::Count(Some(inner))` branch, added a fast path for bound
node/edge variables. Since MATCH only produces bindings where matched variables are
always non-null, `COUNT(n)` where `n` is a node variable can short-circuit to
`bindings_list.len()` without evaluating each binding.

Detection: checks the first binding's value for the variable name — if it resolves to
`BindingValue::Node` or `BindingValue::Edge`, skips per-row evaluation entirely.

### 2. Proper GROUP BY (implicit grouping by non-aggregate keys)

**File**: `crates/maharit-query/src/executor.rs` — `build_aggregated_result_set`

Previously returned a single row regardless of non-aggregate items in RETURN. This was
semantically incorrect for queries like:

```cypher
MATCH (n:Person) RETURN n.city, count(n)
```

Which should return one row per distinct `n.city` value.

New implementation:
- Separates return items into group key indices (non-aggregate) and aggregate indices
- If no group keys: falls through to existing single-row behavior (backward compatible)
- If group keys exist: groups bindings by serialized key values using insertion-order
  preserving `Vec` + `HashMap`, then computes aggregates per group
- Also applies ORDER BY / SKIP / LIMIT on the grouped rows

### 3. COUNT column name fix

`return_item_to_column_name` for `Count(Some(inner))` now returns `"count(<inner>)"`
instead of the incorrect `"COUNT(*)"`. `COUNT(*)` (no inner) still returns `"COUNT(*)"`.

## Tests Added

- `test_count_node_optimization`: 3 Person nodes, verifies `count(n)` returns 3
- `test_group_by_count`: 2 Tokyo + 1 Osaka nodes, verifies 2 rows with correct columns
- `test_group_by_avg`: Eng (100,200) + Sales (150) nodes, verifies avg(salary) per dept
- `test_simple_count_star`: 2 A nodes, verifies `count(*)` still returns single row with 2
