---
title: Property indexes
parent: Indexes and constraints
nav_order: 1
source_commit: 6787bc36
---

# Property indexes

A property index turns a lookup by a given property from a scan of every node with the label into an index lookup (hash for equality, B-tree for ranges), which makes it faster.

## Creating an index

```cypher
// Create an index on a label and property
CREATE INDEX ON :Person(name)

CREATE INDEX ON :User(email)
```

Syntax: `CREATE INDEX ON :Label(property)`. The Neo4j 4.x+ form `CREATE INDEX FOR (n:Label) ON (n.property)` also works (index names and multiple properties are not supported).

```cypher
CREATE INDEX FOR (n:Person) ON (n.email)
```

> **Note**: when you create an index on existing nodes, every node that has the label at that time is indexed automatically. Nodes added or deleted afterwards, and property or label changes made with `SET` / `REMOVE`, are reflected automatically as well.

## Listing indexes

```cypher
SHOW INDEXES
```

Example output:

```
label   property
------  --------
Person  name
User    email
```

## Dropping an index

```cypher
DROP INDEX ON :Person(name)
```

Syntax: `DROP INDEX ON :Label(property)`

## When indexes are used

Inline property filters (the `{prop: value}` form) use the index automatically.

```cypher
// Uses the index (fastest)
MATCH (n:Person {name: "Alice"}) RETURN n

// Range queries (faster with an index on an Int or Float property)
MATCH (n:Person) WHERE n.age > 30 RETURN n

MATCH (n:Person) WHERE n.age >= 25 AND n.age <= 40 RETURN n
```

## Checking index use with EXPLAIN

`EXPLAIN` shows whether an index is used.

```cypher
EXPLAIN MATCH (n:Person {name: "Alice"}) RETURN n.name
```

Example output when the index is used:

```
Operator                    Est. Rows     Cost Details
------------------------------------------------------------------------
IndexSeek                           1        2 :Person.name
Projection                          1        1 columns: n.name
```

Without an index, `NodeByLabelScan` is shown:

```
Operator                    Est. Rows     Cost Details
------------------------------------------------------------------------
NodeByLabelScan                    11        2 :Person
Projection                         11        1 columns: n.name
```

When a `WHERE` condition uses the index, `IndexRangeScan` is shown:

```
Operator                    Est. Rows     Cost Details
------------------------------------------------------------------------
IndexRangeScan                      2        1 :Person.age
Projection                          2        1
```

## Index design best practices

- **Selective properties**: index properties with many distinct values (names, IDs, and so on)
- **Frequently searched properties**: prioritize properties used often in `WHERE` or inline filters
- **Avoid too many indexes**: too many indexes slow down creating and updating nodes

## Current limitations

- An index covers one label and one property (composite indexes are not supported)
- Relationship property indexes are not supported
- Indexes are chosen by the node's first label (for a node with several labels, only the index on the first label is used)
