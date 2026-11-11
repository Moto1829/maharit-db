---
title: 'Parameters ($param syntax)'
parent: Cypher query language
nav_order: 7
source_commit: 87fee75e
---

# Parameters ($param syntax)

Parameterized queries pass values to a query safely, without building the query string dynamically. They also protect against the Cypher equivalent of SQL injection.

## Basic usage

Refer to a parameter with the `$` prefix.

```cypher
// Find a node with a parameter
MATCH (n:Person {name: $name})
RETURN n

// Use parameters in WHERE
MATCH (n:Person)
WHERE n.age >= $min_age AND n.age <= $max_age
RETURN n.name, n.age
```

Parameter values are passed at execution time (as JSON):
```json
{
  "name": "Alice",
  "min_age": 20,
  "max_age": 40
}
```

## Where parameters can be used

Parameters can be used almost anywhere a value can.

```cypher
// Property values
MATCH (n:Person {name: $name}) RETURN n

// Property values in CREATE
CREATE (n:Person {name: $name, age: $age})

// Updating properties with SET
MATCH (n:Person {name: $name})
SET n.age = $new_age

// LIMIT and SKIP
MATCH (n:Person) RETURN n SKIP $offset LIMIT $count

// List parameters
MATCH (n:Person)
WHERE n.city IN $cities
RETURN n.name
```

Example list parameter:
```json
{
  "cities": ["Tokyo", "Osaka", "Kyoto"]
}
```

## Expanding a map into properties

Pass a map as a parameter and expand it into properties.

```cypher
// Create a node from a map parameter
CREATE (n:Person $props)
RETURN n
```

Parameters:
```json
{
  "props": {
    "name": "Alice",
    "age": 30,
    "city": "Tokyo"
  }
}
```

The same form works for relationships (`-[r:KNOWS $rel]->`), `MATCH`, and `MERGE`. In `MATCH` / `MERGE`, every key of the map becomes an equality condition.

```cypher
MATCH (n:Person $props)
RETURN n.name
```

A map parameter in `SET` updates several properties at once. `+=` keeps existing properties; `=` removes properties that are not in the map.

```cypher
MATCH (n:Person {name: $name})
SET n += $changes
```

### Referring to keys of a parameter

Refer to a key of a map parameter with `$props.name` or `$props['name']`. A missing key gives `null`.

```cypher
CREATE (n:Person {name: $props.name, city: $props.address.city})
```

## UNWIND with parameters

Expand a list parameter with `UNWIND` for bulk inserts.

```cypher
UNWIND $persons AS person
CREATE (n:Person {
  name: person.name,
  age: person.age,
  city: person.city
})
```

Parameters:
```json
{
  "persons": [
    {"name": "Alice", "age": 30, "city": "Tokyo"},
    {"name": "Bob", "age": 25, "city": "Osaka"},
    {"name": "Charlie", "age": 35, "city": "Kyoto"}
  ]
}
```

## From the Rust client

```rust
use maharit_client::Client;
use std::collections::HashMap;
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Client::connect("127.0.0.1:7687").await?;

    // Build the parameters
    let mut params: HashMap<String, Value> = HashMap::new();
    params.insert("name".to_string(), json!("Alice"));
    params.insert("min_age".to_string(), json!(25));

    // Run a query with parameters
    let result = client.query_with_params(
        "MATCH (n:Person {name: $name}) WHERE n.age >= $min_age RETURN n.name, n.age",
        params
    ).await?;

    for row in &result.rows {
        println!("{:?}", row);
    }

    Ok(())
}
```

## From the Python client

```python
from maharit import Client

with Client.connect("localhost:7687") as client:
    result = client.query(
        "MATCH (n:Person {name: $name}) WHERE n.age >= $min_age RETURN n.name, n.age",
        {"name": "Alice", "min_age": 25},
    )
```

When sending over TCP directly, put them in the `params` field of the `query` request (see the [TCP protocol specification](../api/protocol.md)).

## Parameter types

These types can be passed as parameters.

| Rust/JSON type | Cypher type |
|----------------|--------------|
| `null` | `null` |
| `bool` | Boolean |
| `i64` / `u64` | Integer |
| `f64` | Float |
| `String` | String |
| `Array` | List |
| `Object` | Map |

## Notes

- Use letters, digits, and underscores in parameter names
- Property keys (a form like `n.$key`) cannot be parameterized
- Label names and relationship types cannot be parameterized either (the structure of a query must be fixed when it is compiled)
- Omitting a parameter that the query refers to is an error (it does not become `null`)
- List and map values can be used with `IN $names`, `UNWIND $rows`, and so on, but they cannot be stored as property values yet
