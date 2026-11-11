---
title: Predicate functions
parent: Function reference
nav_order: 4
source_commit: 733fcd5b
---

# Predicate functions

Predicate functions return a boolean for a list or a value. Use them in `WHERE` clauses and list comprehensions.

## List predicates

### all(variable IN list WHERE condition)

Returns `true` if every element of the list satisfies the condition.

```cypher
RETURN all(x IN [2, 4, 6, 8] WHERE x % 2 = 0)
// Result: true

RETURN all(x IN [1, 2, 3] WHERE x > 0)
// Result: true

RETURN all(x IN [1, 2, 3] WHERE x > 1)
// Result: false

// Check that every node on a path satisfies a condition
MATCH p = (origin:City)-[:ROAD*]->(dest:City)
WHERE all(city IN nodes(p) WHERE city.accessible = true)
RETURN origin.name, dest.name
```

### any(variable IN list WHERE condition)

Returns `true` if at least one element of the list satisfies the condition.

```cypher
RETURN any(x IN [1, 2, 3] WHERE x > 2)
// Result: true

RETURN any(x IN [1, 2, 3] WHERE x > 5)
// Result: false

// Does the person have a friend living in Tokyo?
MATCH (p:Person)-[:KNOWS]->(f:Person)
WITH p, collect(f) AS friends
WHERE any(friend IN friends WHERE friend.city = "Tokyo")
RETURN p.name
```

### none(variable IN list WHERE condition)

Returns `true` if no element of the list satisfies the condition. Equivalent to `NOT any(...)`.

```cypher
RETURN none(x IN [1, 2, 3] WHERE x > 5)
// Result: true

RETURN none(x IN [1, 2, 3] WHERE x > 2)
// Result: false

// Only paths without blocked users
MATCH p = (a:Person)-[:KNOWS*]->(b:Person)
WHERE none(n IN nodes(p) WHERE n.blocked = true)
RETURN a.name, b.name
```

### single(variable IN list WHERE condition)

Returns `true` if exactly one element of the list satisfies the condition.

```cypher
RETURN single(x IN [1, 2, 3] WHERE x = 2)
// Result: true

RETURN single(x IN [1, 2, 3] WHERE x > 1)
// Result: false (two elements satisfy it)

// Check that there is exactly one admin
MATCH (dept:Department {name: "Engineering"})
WITH dept, [(dept)-[:HAS_MEMBER]->(m:Person) | m] AS members
WHERE single(m IN members WHERE m.role = "admin")
RETURN dept.name
```

## Existence checks

### exists(expression)

Checks whether a property or a pattern exists.

```cypher
// Property existence
MATCH (n:Person)
WHERE exists(n.email)
RETURN n.name, n.email

// Pattern existence
MATCH (n:Person)
WHERE exists((n)-[:FOLLOWS]->(:Person))
RETURN n.name

// In RETURN
MATCH (n:Person)
RETURN n.name, exists(n.email) AS has_email
```

Note: `IS NOT NULL` also checks whether a property exists.

```cypher
// These two give the same result
MATCH (n:Person)
WHERE exists(n.email)
RETURN n.name

MATCH (n:Person)
WHERE n.email IS NOT NULL
RETURN n.name
```

### EXISTS {} subqueries (pattern existence)

For more complex patterns, use an `EXISTS {}` subquery.

```cypher
MATCH (p:Person)
WHERE EXISTS {
  MATCH (p)-[:KNOWS]->(f:Person)
  WHERE f.city = "Tokyo"
}
RETURN p.name
```

## Emptiness checks

### isEmpty(value)

Checks whether a list or a string is empty.

```cypher
// Is the list empty?
RETURN isEmpty([])
// Result: true

RETURN isEmpty([1, 2, 3])
// Result: false

// Is the string empty?
RETURN isEmpty("")
// Result: true

RETURN isEmpty("hello")
// Result: false

// Find nodes whose list property is not empty
MATCH (n:Article)
WHERE NOT isEmpty(n.tags)
RETURN n.title, n.tags
```

## Practical examples

```cypher
// Users with every required field set
MATCH (u:User)
WHERE all(field IN ["name", "email", "created_at"] WHERE exists(u[field]))
RETURN u

// Engineers with at least one matching skill
MATCH (e:Engineer)
WHERE any(skill IN e.skills WHERE skill IN ["Rust", "Go", "C++"])
RETURN e.name, e.skills

// The start person appears only once on the path (simple path check)
MATCH p = (a:Person)-[:KNOWS*1..5]->(b:Person)
WHERE a <> b
  AND single(n IN nodes(p) WHERE n = a)
RETURN a.name, b.name, length(p) AS hops
```
