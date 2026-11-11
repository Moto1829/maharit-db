---
title: Schema constraints
parent: Indexes and constraints
nav_order: 3
source_commit: 52f38528
---

# Schema constraints

Schema constraints keep your data consistent. Operations that violate a constraint fail with an error.

## CREATE CONSTRAINT

The syntax is `CREATE CONSTRAINT <name> FOR (<variable>:<Label>) REQUIRE <condition>` (the same form as Neo4j 5). The name is required and is used to drop the constraint with `DROP CONSTRAINT`.

{: .note }
> The old Neo4j 3.x syntax `CREATE CONSTRAINT ON (n:Label) ASSERT ...` / `DROP CONSTRAINT ON ... ASSERT ...` is not supported. Rewrite it with `FOR ... REQUIRE`.

### UNIQUE constraints

Guarantee that a property's value is unique among nodes with a given label.

```cypher
// UNIQUE constraint on one property
CREATE CONSTRAINT personEmailUnique FOR (n:Person) REQUIRE n.email IS UNIQUE

// Composite UNIQUE constraint (the combination of properties is unique)
CREATE CONSTRAINT productSkuWarehouse FOR (n:Product) REQUIRE (n.sku, n.warehouse) IS UNIQUE
```

With a UNIQUE constraint in place, creating a node with a duplicate value fails:

```cypher
CREATE (n:Person {email: "alice@example.com"})
// The second run fails: unique constraint violation: personEmailUnique - property 'email' ...
```

### NOT NULL constraints

Guarantee that a property is never `null`.

```cypher
CREATE CONSTRAINT personNameNotNull FOR (n:Person) REQUIRE n.name IS NOT NULL

CREATE CONSTRAINT productPriceNotNull FOR (n:Product) REQUIRE n.price IS NOT NULL
```

### Type constraints

Guarantee that a property's value has a given type. Specify the type with `IS :: <type>`.

```cypher
// Integer
CREATE CONSTRAINT personAgeInt FOR (n:Person) REQUIRE n.age IS :: INTEGER

// String
CREATE CONSTRAINT personNameString FOR (n:Person) REQUIRE n.name IS :: STRING

// Boolean
CREATE CONSTRAINT settingEnabledBool FOR (n:Setting) REQUIRE n.enabled IS :: BOOLEAN

// Floating point
CREATE CONSTRAINT productPriceFloat FOR (n:Product) REQUIRE n.price IS :: FLOAT
```

Supported types: `INTEGER` (`INT`), `FLOAT` (`DOUBLE`), `STRING`, `BOOLEAN` (`BOOL`)

### Label constraints

Guarantee that nodes with one label also have another label.

```cypher
// Nodes labeled Employee must also be labeled Person
CREATE CONSTRAINT employeeIsPerson FOR (n:Employee) REQUIRE n:Person
```

### Relationship endpoint constraints

Guarantee that relationships of a given type connect nodes with specific labels.

```cypher
// WORKS_AT may only go from Person to Company
CREATE CONSTRAINT worksAtEndpoints FOR (p:Person)-[r:WORKS_AT]->(c:Company)
```

## DROP CONSTRAINT

Drop a constraint by name.

```cypher
DROP CONSTRAINT personEmailUnique
```

## SHOW CONSTRAINTS

Lists all defined constraints.

```cypher
SHOW CONSTRAINTS
```

Example output:

```
+---------------------+---------+----------------+--------------+
| name                | label   | properties     | type         |
+---------------------+---------+----------------+--------------+
| personEmailUnique   | Person  | email          | UNIQUE       |
| personNameNotNull   | Person  | name           | NOT NULL     |
| personAgeInt        | Person  | age            | TYPE(INTEGER)|
| productSkuWarehouse | Product | sku, warehouse | UNIQUE       |
+---------------------+---------+----------------+--------------+
```

## MERGE and constraints

`MERGE` is especially useful with patterns keyed on a property that has a UNIQUE constraint.

```cypher
// Upsert when email is guaranteed to be unique
MERGE (u:User {email: $email})
ON CREATE SET u.created_at = $now, u.name = $name
ON MATCH SET u.last_login = $now
RETURN u
```

## Constraints and indexes

Creating a UNIQUE constraint does not create a property index automatically. If you search by the constrained property, create an index explicitly.

```cypher
CREATE CONSTRAINT personEmailUnique FOR (n:Person) REQUIRE n.email IS UNIQUE

CREATE INDEX ON :Person(email)
```

## Handling constraint violations

With the Rust client, handle constraint violations through `Result`.

```rust
match client.execute("CREATE (n:Person {email: \"alice@example.com\"})").await {
    Ok(_) => println!("Created successfully"),
    Err(e) if e.to_string().contains("unique constraint violation") => {
        println!("Email already exists");
    }
    Err(e) => eprintln!("Error: {}", e),
}
```

## Best practices

- Put UNIQUE constraints on business keys (email addresses, SKUs, user IDs, and so on)
- Put NOT NULL constraints on required fields to keep data quality high
- Use type constraints for type safety
- Combine constraints with `MERGE` for atomic upserts
