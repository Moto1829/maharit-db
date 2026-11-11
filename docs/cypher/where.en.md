---
title: "WHERE clause and filtering"
parent: Cypher query language
nav_order: 2
source_commit: e4cf7500
---

# WHERE clause and filtering

Use the `WHERE` clause to filter the nodes and relationships matched by `MATCH`.

## Basic usage

```cypher
MATCH (n:Person)
WHERE n.age > 25
RETURN n.name, n.age
```

Write `WHERE` right after `MATCH` or `WITH`.

## Comparison operators

| Operator | Meaning | Example |
|--------|------|----|
| `=` | Equal to | `n.age = 30` |
| `<>` | Not equal to | `n.age <> 30` |
| `<` | Less than | `n.age < 30` |
| `<=` | Less than or equal to | `n.age <= 30` |
| `>` | Greater than | `n.age > 30` |
| `>=` | Greater than or equal to | `n.age >= 30` |

```cypher
// People aged 25 to 40
MATCH (n:Person)
WHERE n.age >= 25 AND n.age <= 40
RETURN n.name

// Everyone except a specific name
MATCH (n:Person)
WHERE n.name <> "Alice"
RETURN n.name
```

## Logical operators

| Operator | Meaning |
|--------|------|
| `AND` | Logical AND |
| `OR` | Logical OR |
| `NOT` | Negation |
| `XOR` | Exclusive OR |

```cypher
// Using AND
MATCH (n:Person)
WHERE n.age > 20 AND n.city = "Tokyo"
RETURN n.name

// Using OR
MATCH (n:Person)
WHERE n.city = "Tokyo" OR n.city = "Osaka"
RETURN n.name

// Using NOT
MATCH (n:Person)
WHERE NOT n.name = "Bob"
RETURN n.name

// Combined conditions
MATCH (n:Person)
WHERE (n.age > 30 OR n.role = "admin") AND NOT n.deleted = true
RETURN n.name

// Using XOR (exactly one of the two holds)
MATCH (n:Person)
WHERE n.city = "Tokyo" XOR n.age > 30
RETURN n.name
```

Precedence from highest to lowest: `NOT`, `AND`, `XOR`, `OR`.

## String predicates

```cypher
// Prefix match
MATCH (n:Person)
WHERE n.name STARTS WITH "Al"
RETURN n.name

// Suffix match
MATCH (n:Person)
WHERE n.email ENDS WITH "@example.com"
RETURN n.name

// Substring match
MATCH (n:Person)
WHERE n.bio CONTAINS "engineer"
RETURN n.name
```

## IN operator

Checks whether a value equals any value in a list.

```cypher
MATCH (n:Person)
WHERE n.city IN ["Tokyo", "Osaka", "Kyoto"]
RETURN n.name, n.city
```

## IS NULL / IS NOT NULL

Checks whether a property is set.

```cypher
// Users without an email
MATCH (n:Person)
WHERE n.email IS NULL
RETURN n.name

// Users with an email
MATCH (n:Person)
WHERE n.email IS NOT NULL
RETURN n.name, n.email
```

## Pattern conditions

A pattern in `WHERE` checks whether that pattern exists.

```cypher
// Users who follow someone
MATCH (n:Person)
WHERE (n)-[:FOLLOWS]->(:Person)
RETURN n.name

// Users who have not blocked anyone
MATCH (n:Person)
WHERE NOT (n)-[:BLOCKED]->(:Person)
RETURN n.name
```

## Label checks

```cypher
// Check whether a node has a given label
MATCH (n)
WHERE n:Person OR n:Company
RETURN n.name
```

## Ranges

```cypher
// Check that a property is within a range
MATCH (p:Product)
WHERE 100 <= p.price <= 1000
RETURN p.name, p.price
```

## Pattern properties versus WHERE

For equality conditions on properties, writing them inside the pattern and writing them in `WHERE` are equivalent.

```cypher
// Filter in the pattern (concise)
MATCH (n:Person {name: "Alice"}) RETURN n

// WHERE clause (sometimes easier to read)
MATCH (n:Person)
WHERE n.name = "Alice"
RETURN n
```

Conditions other than equality (`>`, `<`, `STARTS WITH`, and so on) always go in `WHERE`.
