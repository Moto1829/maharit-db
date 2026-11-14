---
title: Basic syntax
parent: Cypher query language
nav_order: 1
source_commit: 52f38528
---

# Cypher basic syntax

MaharitDB supports a query language with syntax close to Neo4j's Cypher. This page covers the most basic query patterns.

## Node patterns

Nodes are written in parentheses `()`.

```text
// Node without a label
()

// Node with a label
(:Person)

// Bind to a variable
(n:Person)

// Property filter
(n:Person {name: "Alice"})

// Multiple labels
(n:Person:Employee)
```

### Multiple labels

A node can have multiple labels. A label represents a category the node belongs to, and a node can belong to several categories at once.

```cypher
// Create a node with multiple labels
CREATE (n:Person:Employee {name: "Alice", department: "Engineering"})

// Match on multiple labels (AND — only nodes that have both labels)
MATCH (n:Person:Employee) RETURN n.name

// Match on just one of the labels
MATCH (n:Employee) RETURN n.name
```

When you specify multiple labels in MATCH, only **nodes that have all of the labels** are returned (AND, not OR).

```cypher
// Get the list of labels
MATCH (n:Person) RETURN n.name, labels(n)
// Result: a list such as ["Person", "Employee"]
```

## Edge (relationship) patterns

Edges are written with square brackets `[]` and arrows.

```text
// Directed edge
(a)-[:KNOWS]->(b)

// Reverse direction
(a)<-[:KNOWS]-(b)

// Undirected (matching only)
(a)-[:KNOWS]-(b)

// Variable binding and properties
(a)-[r:KNOWS {since: 2020}]->(b)

// Several types (matches any of them; matching only)
(a)-[:KNOWS|FOLLOWS]->(b)

// Number of hops (variable length)
(a)-[:KNOWS*1..3]->(b)
```

An edge created by CREATE or MERGE takes exactly one type (`[:A|B]` is an error).

## CREATE

Creates nodes and edges.

```cypher
// Create a node
CREATE (n:Person {name: "Alice", age: 30})

// Create an edge (referring to existing nodes)
MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"})
CREATE (a)-[:KNOWS {since: 2021}]->(b)

// Create several nodes at once
CREATE
  (alice:Person {name: "Alice"}),
  (bob:Person {name: "Bob"}),
  (alice)-[:FRIENDS]->(bob)
```

## MATCH

Finds data in the graph that matches a pattern.

```cypher
// Get all nodes
MATCH (n) RETURN n

// Get nodes with a specific label
MATCH (n:Person) RETURN n

// Filter by property
MATCH (n:Person {name: "Alice"}) RETURN n

// Pattern including an edge
MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b

// Variable-length path
MATCH (a:Person)-[:KNOWS*1..3]->(b:Person)
WHERE a.name = "Alice"
RETURN b.name
```

## RETURN

Specifies the values returned as the query result.

```cypher
// Return whole nodes
MATCH (n:Person) RETURN n

// Return properties
MATCH (n:Person) RETURN n.name, n.age

// Use aliases
MATCH (n:Person) RETURN n.name AS name, n.age AS age

// Return a computed expression
MATCH (n:Person) RETURN n.name, n.age * 2 AS double_age

// Return constants
RETURN 42, "hello", true

// DISTINCT (remove duplicates)
MATCH (n:Person) RETURN DISTINCT n.name
```

## DELETE and DETACH DELETE

Deletes nodes and edges.

```cypher
// Delete an edge
MATCH (a:Person)-[r:KNOWS]->(b:Person)
WHERE a.name = "Alice" AND b.name = "Bob"
DELETE r

// Delete a node (only if it has no edges)
MATCH (n:Person {name: "Charlie"})
DELETE n

// Delete a node and all edges connected to it
MATCH (n:Person {name: "Dave"})
DETACH DELETE n
```

## Comments

Everything from `//` to the end of the line is a comment. Text enclosed in `/* … */` is also a comment and may span multiple lines.

```cypher
// This is a comment
MATCH (n:Person) // Inline comments work too
/* A block comment
   spanning lines */
RETURN n.name
```

{: .note }
> SQL-style `--` is not treated as a comment (to distinguish it from undirected patterns such as `(a)--(b)`).

## Multiple statements

A single request can run only one statement. Sending several statements at once separated by semicolons (`;`) is not supported. Run multiple operations one at a time.

```cypher
CREATE (a:Person {name: "Alice"})

CREATE (b:Person {name: "Bob"})

MATCH (a:Person), (b:Person)
WHERE a.name <> b.name
CREATE (a)-[:KNOWS]->(b)
```

## Literal values

| Type | Examples |
|---|---|
| Integer | `42`, `-10`, `0` |
| Floating point | `3.14`, `-0.5`, `1.0e3` |
| String | `"hello"`, `"日本語"` |
| Boolean | `true`, `false` |
| null | `null` |
| List | `[1, 2, 3]`, `["a", "b"]` |
| Map | `{name: "Alice", age: 30}` |
