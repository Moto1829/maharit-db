---
title: FOREACH / subqueries
parent: Cypher query language
nav_order: 5
source_commit: 52f38528
---

# FOREACH / subqueries

## FOREACH

`FOREACH` runs update operations (`CREATE`, `SET`, `MERGE`, `DELETE`, `REMOVE`) once for each element of a list. `RETURN` cannot be used inside it.

### Basic usage

```cypher
// Create a node for each element of a list
FOREACH (name IN ["Alice", "Bob", "Charlie"] |
  CREATE (:Person {name: name})
)
```

### Updating every node on a path

```cypher
// Set a property on every node on the path
MATCH p = (start:Person {name: "Alice"})-[:KNOWS*]->(target:Person)
FOREACH (n IN nodes(p) |
  SET n.visited = true
)
```

### Combining with MATCH

```cypher
// Expand a list of tags and create relationships
MATCH (a:Article {id: $id})
FOREACH (tag IN $tags |
  MERGE (t:Tag {name: tag})
  CREATE (a)-[:TAGGED_WITH]->(t)
)
```

### Nested FOREACH

```cypher
// Process a list of lists
FOREACH (row IN [[1, "Alice"], [2, "Bob"]] |
  FOREACH (item IN row |
    // Inner operation (of limited practical use)
    CREATE (:Log {value: item})
  )
)
```

### Conditional updates (with CASE)

`FOREACH` can also stand in for a conditional. To run an update only when a condition holds:

```cypher
// Run only when the condition holds (CASE makes the list length 0 or 1)
MATCH (n:Person)
FOREACH (_ IN CASE WHEN n.age >= 18 THEN [1] ELSE [] END |
  SET n:Adult
)
```

## CALL {} subqueries

Write a subquery inside a `CALL {}` block. The subquery can refer to bindings from the outer query (imported variables).

### Basic usage

```cypher
// A subquery independent of the outer query
CALL {
  MATCH (n:Person)
  RETURN count(n) AS person_count
}
RETURN person_count
```

### Importing outer variables

```cypher
// Refer to a variable from the outer query
MATCH (p:Person)
CALL {
  WITH p
  MATCH (p)-[:FRIENDS]->(f:Person)
  RETURN count(f) AS friend_count
}
RETURN p.name, friend_count
```

### Aggregating in a subquery

```cypher
// Count each person's friends
MATCH (p:Person)
CALL {
  WITH p
  MATCH (p)-[:KNOWS]->(friend:Person)
  RETURN count(friend) AS num_friends,
         collect(friend.name) AS friend_names
}
RETURN p.name, num_friends, friend_names
ORDER BY num_friends DESC
```

### EXISTS {} subqueries

Checks whether a pattern exists (inside `WHERE`).

```cypher
// Return only people who have followers
MATCH (p:Person)
WHERE EXISTS {
  MATCH (p)<-[:FOLLOWS]-(:Person)
}
RETURN p.name
```

### COUNT {} subqueries

Counts the rows of a subquery.

```cypher
// Get the number of followers
MATCH (p:Person)
RETURN p.name,
       COUNT { MATCH (p)<-[:FOLLOWS]-(:Person) } AS follower_count
ORDER BY follower_count DESC
```

### COLLECT {} subqueries

Collects the results of a subquery into a list.

```cypher
// Get each person's friends' names as a list
MATCH (p:Person)
RETURN p.name,
       COLLECT { MATCH (p)-[:KNOWS]->(f:Person) RETURN f.name } AS friends
```

### UNION in subqueries

`UNION` can be used inside `CALL {}`.

```cypher
MATCH (p:Person {name: "Alice"})
CALL {
  WITH p
  MATCH (p)-[:FOLLOWS]->(f:Person)
  RETURN f.name AS name, "follows" AS rel_type
  UNION
  WITH p
  MATCH (p)-[:KNOWS]->(f:Person)
  RETURN f.name AS name, "knows" AS rel_type
}
RETURN name, rel_type
```
