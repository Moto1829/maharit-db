---
title: SET / REMOVE / MERGE
parent: Cypher query language
nav_order: 3
source_commit: 52f38528
---

# SET / REMOVE / MERGE

## SET

`SET` sets or updates properties on nodes and relationships.

### Basic usage

```cypher
// Set a property
MATCH (n:Person {name: "Alice"})
SET n.age = 31

// Set several properties at once
MATCH (n:Person {name: "Alice"})
SET n.age = 31, n.email = "alice@example.com"

// Set a relationship property
MATCH (a:Person)-[r:KNOWS]->(b:Person)
WHERE a.name = "Alice" AND b.name = "Bob"
SET r.since = 2022
```

### Adding labels

`SET n:Label` adds a label to a node. Existing labels are kept.

```cypher
// Add a label (existing labels are kept)
MATCH (n:Person {name: "Alice"})
SET n:Admin
// Alice now has both Person and Admin

// Add several labels at once
MATCH (n:Person {name: "Alice"})
SET n:Admin:Moderator
// Alice now has Person, Admin, and Moderator
```

Check the resulting labels with `labels(n)`.

```cypher
MATCH (n:Person {name: "Alice"})
SET n:Admin
RETURN n.name, labels(n)
// Result: ["Person", "Admin"]
```

### Setting properties from a map

```cypher
// Update from a map (existing properties are kept)
MATCH (n:Person {name: "Alice"})
SET n += {age: 32, city: "Tokyo"}

// Replace all properties with a map (existing properties are removed)
MATCH (n:Person {name: "Alice"})
SET n = {name: "Alice", age: 32}
```

### MATCH + SET

```cypher
// Filter, then update
MATCH (n:Person)
WHERE n.age < 18
SET n:Minor

// Update with an expression
MATCH (n:Product)
SET n.price = n.price * 1.1
```

## REMOVE

`REMOVE` removes properties and labels.

### Removing properties

```cypher
// Remove a property
MATCH (n:Person {name: "Alice"})
REMOVE n.temporary_flag

// Remove several properties at once
MATCH (n:Person {name: "Alice"})
REMOVE n.flag1, n.flag2
```

### Removing labels

`REMOVE n:Label` removes one label. Other labels are kept.

```cypher
// Remove a label (other labels are kept)
MATCH (n:Person:Temp {name: "Alice"})
REMOVE n:Temp
// Alice now has only Person

// Remove several labels at once
MATCH (n)
WHERE n:Draft AND n:Review
REMOVE n:Draft, n:Review
SET n:Published
```

## MERGE

`MERGE` creates a pattern only if it does not exist yet, and otherwise uses the existing one. It works much like an "upsert".

### Basic usage

```cypher
// Create if it does not exist
MERGE (n:Person {name: "Alice"})

// Branch on whether it was created or found
MERGE (n:Person {name: "Alice"})
ON CREATE SET n.created_at = 2024, n.age = 30
ON MATCH SET n.updated_at = 2024
```

### MERGE on relationships

```cypher
// Create the relationship if it does not exist
MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"})
MERGE (a)-[:KNOWS]->(b)

// With ON CREATE / ON MATCH
MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"})
MERGE (a)-[r:KNOWS]->(b)
ON CREATE SET r.since = 2024
ON MATCH SET r.last_seen = 2024
```

### MERGE on a whole pattern

```cypher
// Merge a whole pattern including nodes and a relationship
MERGE (a:Person {name: "Alice"})-[:WORKS_AT]->(c:Company {name: "Acme"})
```

Note: `MERGE` checks whether the whole pattern matches. If only part of the pattern exists, it is treated as missing and the whole pattern is created. For complex patterns, `MERGE` the nodes and the relationships separately.

### A typical MERGE use case

```cypher
// Record a user login (create the node the first time, update it afterwards)
MERGE (u:User {email: $email})
ON CREATE SET
  u.created_at = $now,
  u.login_count = 1
ON MATCH SET
  u.last_login = $now,
  u.login_count = u.login_count + 1
RETURN u
```

## Choosing between SET, REMOVE, and MERGE

| Operation | Syntax |
|------|------------|
| Add or update a property | `SET` |
| Remove a property | `REMOVE` |
| Add a label | `SET n:Label` |
| Remove a label | `REMOVE n:Label` |
| Create if missing (upsert) | `MERGE` |
| Different updates on create and on match | `MERGE ... ON CREATE ... ON MATCH` |
