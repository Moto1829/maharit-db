---
title: UNWIND / WITH / UNION
parent: Cypher query language
nav_order: 4
source_commit: 52f38528
---

# UNWIND / WITH / UNION

## UNWIND

`UNWIND` expands a list so that each element becomes its own row. It takes a list as input and runs the rest of the query once per element.

### Basic usage

```cypher
// Expand a list literal
UNWIND [1, 2, 3] AS num
RETURN num

// Result:
// num: 1
// num: 2
// num: 3
```

### Expanding a list property

```cypher
// Expand a node's list of tags
MATCH (a:Article)
UNWIND a.tags AS tag
RETURN a.title, tag
```

### Bulk inserts

```cypher
// Create nodes from a list passed as a parameter
UNWIND $persons AS person
CREATE (n:Person {name: person.name, age: person.age})
```

Example parameters (JSON):
```json
{
  "persons": [
    {"name": "Alice", "age": 30},
    {"name": "Bob", "age": 25},
    {"name": "Charlie", "age": 35}
  ]
}
```

### UNWIND with MATCH

```cypher
// Look up nodes from a list of IDs
UNWIND [1, 2, 3] AS id
MATCH (n:Product)
WHERE n.id = id
RETURN n.name, n.price
```

### Expanding nested lists

```cypher
// Expand a list of lists
UNWIND [[1, 2], [3, 4], [5, 6]] AS sublist
UNWIND sublist AS num
RETURN num
```

## WITH

`WITH` splits a query into stages and passes intermediate results to the next stage. Use it to filter on aggregated values or to structure a query into logical steps.

### Filtering intermediate results

```cypher
// People with three or more friends
MATCH (p:Person)-[:FRIENDS]->(f:Person)
WITH p, count(f) AS friend_count
WHERE friend_count >= 3
RETURN p.name, friend_count
ORDER BY friend_count DESC
```

### Working with aggregated values

```cypher
// People older than the average age
MATCH (p:Person)
WITH avg(p.age) AS avg_age
MATCH (p:Person)
WHERE p.age > avg_age
RETURN p.name, p.age, avg_age
```

### Pipelines

```cypher
// Stage 1: filter
MATCH (p:Person)-[:WORKS_AT]->(c:Company)
WHERE c.industry = "Tech"
WITH p, c
// Stage 2: add more conditions
MATCH (p)-[:KNOWS]->(colleague:Person)
WITH p, c, collect(colleague.name) AS colleagues
// Stage 3: return the result
RETURN p.name, c.name AS company, colleagues
ORDER BY p.name
LIMIT 10
```

### Variable scope

Variables not listed in `WITH` are not available in later stages.

```cypher
MATCH (p:Person), (c:Company)
WHERE p.employer = c.name
WITH p  // c is dropped here
RETURN p.name
// c cannot be referenced here
```

## UNION

`UNION` combines the results of several queries. By default it removes duplicates.

### Basic usage

```cypher
// Combine two query results (duplicates removed)
MATCH (n:Person) RETURN n.name AS name
UNION
MATCH (n:Company) RETURN n.name AS name
```

### UNION ALL (keep duplicates)

```cypher
// Combine and keep duplicates
MATCH (n:Person) RETURN n.name AS name, "person" AS type
UNION ALL
MATCH (n:Company) RETURN n.name AS name, "company" AS type
```

### Combining several queries

```cypher
// Combine queries with different conditions
MATCH (n:Person) WHERE n.city = "Tokyo" RETURN n.name AS name, n.city AS location
UNION
MATCH (n:Person) WHERE n.city = "Osaka" RETURN n.name AS name, n.city AS location
UNION
MATCH (n:Person) WHERE n.city = "Kyoto" RETURN n.name AS name, n.city AS location
```

Note: every query combined with `UNION` must return the same number of columns with the same names.

### UNION and ORDER BY

An `ORDER BY` / `LIMIT` written after the last query applies only to that query (its `RETURN`), as in Cypher. To sort the combined result, wrap the `UNION` in `CALL { … }` and write `ORDER BY` outside it.

```cypher
CALL {
  MATCH (n:Person) RETURN n.name AS name
  UNION
  MATCH (n:Company) RETURN n.name AS name
}
RETURN name
ORDER BY name ASC
LIMIT 20
```
