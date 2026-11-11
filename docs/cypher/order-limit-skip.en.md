---
title: ORDER BY / LIMIT / SKIP
parent: Cypher query language
nav_order: 6
source_commit: d3e44d25
---

# ORDER BY / LIMIT / SKIP

## ORDER BY

`ORDER BY` sorts the results. Write it after `RETURN`.

### Basic usage

```cypher
// Ascending by name (the default)
MATCH (n:Person)
RETURN n.name, n.age
ORDER BY n.name

// Descending by age
MATCH (n:Person)
RETURN n.name, n.age
ORDER BY n.age DESC

// Explicitly ascending
MATCH (n:Person)
RETURN n.name, n.age
ORDER BY n.age ASC
```

### Sorting by several keys

```cypher
// Ascending by city, then descending by age within a city
MATCH (n:Person)
RETURN n.name, n.city, n.age
ORDER BY n.city ASC, n.age DESC
```

### Sorting by an alias

```cypher
// Aliases defined in RETURN can be used for sorting
MATCH (n:Person)-[:KNOWS]->(f:Person)
RETURN n.name AS name, count(f) AS friend_count
ORDER BY friend_count DESC, name ASC
```

### Null values

When sorting, `null` values come last in ascending order and first in descending order.

```cypher
MATCH (n:Person)
RETURN n.name, n.score
ORDER BY n.score DESC
// Nodes whose score is null come first
```

## LIMIT

`LIMIT` sets the maximum number of rows to return.

```cypher
// Return the first 10 rows
MATCH (n:Person)
RETURN n.name, n.age
LIMIT 10

// Combine with ORDER BY to get the top rows
MATCH (n:Person)
RETURN n.name, n.age
ORDER BY n.age DESC
LIMIT 5
```

### LIMIT as a parameter

```cypher
MATCH (n:Person)
RETURN n.name
LIMIT $limit
```

Besides integer literals and parameters, `SKIP` / `LIMIT` accept expressions built from them (such as `$size * ($page - 1)`). The value must be a non-negative integer, and values that vary per row, such as nodes or properties, cannot be used.

## SKIP

`SKIP` skips the given number of rows before returning results. It is used for pagination.

```cypher
// Skip the first 10 rows and return the next 10 (page 2)
MATCH (n:Person)
RETURN n.name, n.age
ORDER BY n.name
SKIP 10
LIMIT 10
```

### Pagination

```cypher
// Take the page number and page size as parameters
MATCH (n:Person)
RETURN n.name, n.age
ORDER BY n.name
SKIP $page_size * ($page - 1)
LIMIT $page_size
```

Example parameters:
```json
{
  "page": 2,
  "page_size": 10
}
```

## Clause order

`ORDER BY`, `SKIP`, and `LIMIT` are written in this order:

```
RETURN ...
ORDER BY ...
SKIP ...
LIMIT ...
```

```cypher
// An example combining all of them
MATCH (n:Person)
WHERE n.age >= 18
RETURN n.name, n.age, n.city
ORDER BY n.age DESC, n.name ASC
SKIP 20
LIMIT 10
```

## Using them with WITH

`ORDER BY`, `SKIP`, and `LIMIT` can also follow `WITH`.

```cypher
// Take the top 10 people, then continue
MATCH (p:Person)-[:KNOWS]->(f:Person)
WITH p, count(f) AS friends
ORDER BY friends DESC
LIMIT 10
MATCH (p)-[:WORKS_AT]->(c:Company)
RETURN p.name, friends, c.name AS company
```

## Performance notes

- Applying `LIMIT` as early as possible makes processing more efficient
- With an index, combining `ORDER BY` and `LIMIT` can sometimes stop early
- Use `EXPLAIN` to check the query plan
