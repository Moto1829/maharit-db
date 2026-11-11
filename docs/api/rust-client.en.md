---
title: Rust client
parent: API reference
nav_order: 1
source_commit: 80a2d7b0
---

# Rust client

The `maharit-client` crate is a client for connecting to a MaharitDB server. It provides an async client (`Client`, tokio), a blocking client (`sync::SyncClient`), and a connection pool (`pool::ConnectionPool`).

## Installation

The crate is not published on crates.io; add it as a dependency from the Git repository.

```toml
[dependencies]
maharit-client = { git = "https://github.com/Moto1829/maharit-db" }
tokio = { version = "1", features = ["full"] }
serde_json = "1"
```

## Async client (Client)

### Connecting

```rust
use maharit_client::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Client::connect("127.0.0.1:7687").await?;
    client.ping().await?;
    println!("Connected to MaharitDB");
    Ok(())
}
```

### Running queries

```rust
// A query whose result is not needed
client.execute("CREATE (n:Person {name: \"Alice\", age: 30})").await?;

// A query that returns results
let result = client.query("MATCH (n:Person) RETURN n.name AS name, n.age AS age").await?;
println!("Rows returned: {}", result.row_count());
```

### Handling results

`QueryResult::rows` is a `Vec` of rows; each row is a `HashMap<String, serde_json::Value>` from column name to value.

```rust
for row in &result.rows {
    let name = row.get("name").and_then(|v| v.as_str()).unwrap_or_default();
    let age = row.get("age").and_then(|v| v.as_i64()).unwrap_or(0);
    println!("{name}: {age}");
}

// When you only need one column of the first row
let count = client
    .query("MATCH (n:Person) RETURN count(n) AS c")
    .await?
    .get_single("c")
    .and_then(|v| v.as_i64());
```

### Queries with parameters

Pass values as parameters (`Params` = `HashMap<String, serde_json::Value>`) instead of embedding them in the query string.

```rust
use maharit_client::Params;
use serde_json::json;

let mut params = Params::new();
params.insert("name".to_string(), json!("Alice"));
params.insert("min_age".to_string(), json!(25));

let result = client
    .query_with_params(
        "MATCH (n:Person {name: $name}) WHERE n.age >= $min_age RETURN n.name AS name",
        params,
    )
    .await?;
```

### Bulk loading

```rust
let mut params = Params::new();
params.insert(
    "persons".to_string(),
    json!([
        {"name": "Alice", "age": 30},
        {"name": "Bob", "age": 25},
        {"name": "Charlie", "age": 35}
    ]),
);

client
    .execute_with_params(
        "UNWIND $persons AS person CREATE (n:Person {name: person.name, age: person.age})",
        params,
    )
    .await?;
```

### Receiving results in chunks (streaming)

```rust
let mut stream = client.query_stream("MATCH (n:Person) RETURN n.name AS name", 1000).await?;
while let Some(chunk) = stream.next_chunk().await? {
    for row in chunk {
        println!("{:?}", row.get("name"));
    }
}
```

### Transactions

```rust
let tx = client.begin().await?;
client.query_in_tx("CREATE (:Item {id: 1})", Some(tx)).await?;
client.commit(tx).await?; // client.rollback(tx) to undo
```

See [Transactions](../advanced/transactions.md) for how they behave.

## Blocking client (SyncClient)

In programs without an async runtime, use the blocking client (it runs a tokio runtime internally).

```rust
use maharit_client::sync::SyncClient;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = SyncClient::connect("127.0.0.1:7687")?;

    client.execute("CREATE (n:Person {name: \"Bob\"})")?;

    let result = client.query("MATCH (n:Person) RETURN n.name")?;
    println!("Found {} persons", result.row_count());
    Ok(())
}
```

## Connection pool

When several tasks use the server at the same time, use a connection pool.

```rust
use maharit_client::pool::{ConnectionPool, PoolConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = ConnectionPool::new(
        "127.0.0.1:7687",
        PoolConfig {
            min_connections: 2,
            max_connections: 10,
            ..Default::default()
        },
    )
    .await?;

    // Take a connection from the pool (it goes back when dropped)
    let mut conn = pool.get().await?;
    let result = conn.query("MATCH (n:Person) RETURN count(n) AS c").await?;
    println!("{:?}", result.get_single("c"));
    Ok(())
}
```

## TLS connections

If the server runs with `--tls-cert` / `--tls-key`, connect with `connect_tls` (see [TLS/SSL](../operations/tls.md)).

```rust
use maharit_client::{Client, TlsClientConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The first argument is the host name to verify the certificate against. Without with_ca_cert, public CAs are used
    let tls = TlsClientConfig::new("secure-db.example.com").with_ca_cert("/path/to/ca.crt");
    let mut client = Client::connect_tls("secure-db.example.com:7687", tls).await?;
    client.ping().await?;
    Ok(())
}
```

`reconnect()` is not available on TLS connections. If the connection drops, connect again with `connect_tls`.

## Authentication

If the server runs with `--require-auth`, call `login` after connecting. The session token is then attached to later requests automatically (see [Authentication and roles](../operations/auth.md)).

```rust
let mut client = Client::connect("127.0.0.1:7687").await?;
let info = client.login("alice", "password123").await?;
println!("role: {}, expires at: {}", info.role, info.expires_at);
```

## Error handling

Query errors and authentication errors come back as `ClientError::Server(message)`. A dropped connection is `ClientError::ConnectionClosed` / `ClientError::Connection`.

```rust
use maharit_client::{Client, ClientError};

match client.execute("CREATE (n:Person {email: \"dup@example.com\"})").await {
    Ok(()) => println!("Created"),
    Err(ClientError::Server(msg)) if msg.contains("unique constraint violation") => {
        println!("Email already exists");
    }
    Err(ClientError::ConnectionClosed | ClientError::Connection(_)) => {
        eprintln!("Lost connection to server");
    }
    Err(e) => return Err(e.into()),
}
```
