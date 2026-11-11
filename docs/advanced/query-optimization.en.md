---
title: Query optimization (EXPLAIN / PROFILE)
parent: Advanced topics
nav_order: 4
source_commit: 6787bc36
---

# Query optimization (EXPLAIN / PROFILE)

`EXPLAIN` and `PROFILE` show a query's execution plan (query plan).

## EXPLAIN

Shows the execution plan without running the query. Each line is one operation (operator), executed from top to bottom.

```cypher
EXPLAIN MATCH (p:Person {name: "Alice"})-[:KNOWS]->(f:Person)
RETURN p.name, f.name
```

Example output:

```text
Operator                    Est. Rows     Cost Details
------------------------------------------------------------------------
NodeByLabelScan                     1        1
Expand                              2        1 :KNOWS
Projection                          2        1
```

- `Est. Rows`: the estimated number of rows the operator produces
- `Cost`: the estimated amount of work (relative)
- `Details`: the label, relationship type, aggregation keys, and so on

!!! warning "Known issue"
    `EXPLAIN` / `PROFILE` currently ignore indexes and show `NodeByLabelScan` instead of `IndexSeek` even when an index exists (bug/167).

## PROFILE

Runs the query and shows the execution plan together with the number of result rows and the execution time.

```cypher
PROFILE MATCH (p:Person {name: "Alice"})-[:KNOWS]->(f:Person)
RETURN p.name, f.name
```

Example output:

```text
Operator                    Est. Rows     Cost  Actual Rows    Time (us)
------------------------------------------------------------------------
NodeByLabelScan                     1        1            -            -
Expand                              2        1            -            -
Projection                          2        1            2          133
Rows: 2, Time: 133 us
```

The actual rows and time are for the whole query and are shown on the last operator (there are no per-operator measurements).

## Operators

| Operator | Description |
|------------|------|
| `NodeByLabelScan` | Scan the nodes with a label |
| `NodeByLabelScan+Filter` | Scan by label while filtering on conditions (filter pushdown) |
| `IndexSeek` | Look up an equality condition in a property index |
| `IndexRangeScan` | Look up a range condition in a property index |
| `Expand` / `ExpandReverse` | Follow relationships to neighboring nodes (forward / backward) |
| `Filter` | Filter on conditions |
| `HashAggregation` / `EagerAggregation` | Aggregation |
| `Sort` | Sort |
| `Limit` | Limit the number of rows |
| `Projection` | Build the returned columns |
| `Unwind` / `Union` / `CallSubquery` / `ProcedureCall` | `UNWIND` / `UNION` / `CALL { … }` / `CALL proc()` |
| `CreateNode` / `SetProperty` / `RemoveProperty` / `Delete` / `Merge` / `Foreach` | Write operations |

## Query cache

The server caches parsed queries (syntax trees) by query string, up to 512 entries (least recently used are evicted first). Repeating a query with the same string skips parsing. Passing values as parameters (`$name`) keeps the string the same when the values change, so the cache applies. There is no way to inspect or clear the cache.

## Optimization techniques

### 1. Use indexes

The most effective optimization is using indexes well.

```cypher
// Without an index (NodeByLabelScan + Filter)
MATCH (n:Person) WHERE n.email = "alice@example.com" RETURN n

// Create the index
CREATE INDEX FOR (n:Person) ON (n.email)

// With an index (IndexSeek) → fast
MATCH (n:Person) WHERE n.email = "alice@example.com" RETURN n
```

### 2. Filter pushdown

Filter conditions are applied as early as possible (right after pattern matching).

```cypher
// Recommended: put the condition in the MATCH pattern
MATCH (p:Person {age: 30})-[:KNOWS]->(f:Person)
RETURN f.name

// Works the same way (filter pushdown applies)
MATCH (p:Person)-[:KNOWS]->(f:Person)
WHERE p.age = 30
RETURN f.name
```

### 3. Traverse variable-length paths from the narrow end

Patterns are traversed from the leftmost node. For variable-length paths, putting the node that conditions narrow down to a few on the left reduces the paths explored.

```cypher
// Recommended: start from Alice, narrowed down to one node
MATCH (p:Person {name: "Alice"})-[:KNOWS*1..5]->(f:Person)
RETURN f.name

// Not recommended: start from every Person and narrow down to Alice at the end
MATCH (f:Person)-[:KNOWS*1..5]->(p:Person {name: "Alice"})
RETURN f.name
```

### 4. Limit intermediate results with WITH

Combine `LIMIT` or aggregation with `WITH` to narrow down results early.

```cypher
// Narrow down to the top 100 people, then continue
MATCH (p:Person)
WHERE p.age > 20
WITH p
ORDER BY p.score DESC
LIMIT 100
MATCH (p)-[:KNOWS]->(f:Person)
RETURN p.name, collect(f.name) AS friends
```

### 5. Be careful with variable-length paths

Variable-length paths (`*n..m`) can cause a combinatorial explosion.

```cypher
// Careful: many hops are slow
MATCH (a:Person)-[:KNOWS*1..10]->(b:Person)
RETURN b.name

// Recommended: limit the number of hops
MATCH (a:Person)-[:KNOWS*1..3]->(b:Person)
RETURN b.name
```
