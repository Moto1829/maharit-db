---
title: Quick start
nav_order: 2
source_commit: afbcbdf0
---

# Quick start

This guide walks you through installing MaharitDB and running your first graph queries.

## 1. Install

You need the Rust toolchain (1.75 or later). If you don't have it yet, install it from [rustup.rs](https://rustup.rs).

```bash
# Clone the repository
git clone https://github.com/Moto1829/maharit-db.git
cd maharit-db

# Release build (recommended)
cargo build --release -p maharit-server

# Location of the built binary
ls target/release/maharit
```

## 2. Start the REPL

The REPL (Read-Eval-Print Loop) mode lets you run queries interactively.

```bash
# Start the REPL with a debug build
cargo run -p maharit-server

# Or run the release binary directly
./target/release/maharit
```

On startup it prints the following message, and you can type queries at the `maharit>` prompt. Type `:help` for a list of commands and `:quit` to exit.

```text
MaharitDB - Graph Database v0.2.0
Type :help for available commands

maharit>
```

## 3. Your first queries

Write queries return a table with the number of items created, updated, or deleted.

### Create nodes

```text
maharit> CREATE (alice:Person {name: "Alice", age: 30})
created_nodes | created_edges
--------------------------------
1 | 0

1 row(s) returned
maharit> CREATE (bob:Person {name: "Bob", age: 25})
created_nodes | created_edges
--------------------------------
1 | 0

1 row(s) returned
```

### Create a relationship

```text
maharit> MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) CREATE (a)-[:KNOWS {since: 2020}]->(b)
created_nodes | created_edges
--------------------------------
0 | 1

1 row(s) returned
```

### Query data

```text
maharit> MATCH (n:Person) RETURN n.name, n.age ORDER BY n.name
n.name | n.age
-----------------
"Alice" | 30
"Bob" | 25

2 row(s) returned
```

### Traverse the graph

```text
maharit> MATCH (p:Person)-[:KNOWS]->(friend:Person) RETURN p.name AS person, friend.name AS friend
person | friend
------------------
"Alice" | "Bob"

1 row(s) returned
```

### Update a property

```text
maharit> MATCH (n:Person {name: "Alice"}) SET n.email = "alice@example.com"
properties_set
-----------------
1

1 row(s) returned
```

### Delete a node

`DETACH DELETE` deletes a node together with the relationships connected to it.

```text
maharit> MATCH (n:Person {name: "Bob"}) DETACH DELETE n
deleted_nodes | deleted_edges
--------------------------------
1 | 0

1 row(s) returned
```

!!! note
    In the current implementation, relationships removed along with the node by `DETACH DELETE` are not counted in `deleted_edges` (the relationships themselves are deleted).

## 4. Start in server mode

Run MaharitDB as a TCP server so that clients can connect over the network.

```bash
# Start with the default settings (127.0.0.1:7687)
cargo run -p maharit-server -- server

# Bind to all interfaces
cargo run -p maharit-server -- server --host 0.0.0.0 --port 7687

# Set the maximum number of connections
cargo run -p maharit-server -- server --host 0.0.0.0 --port 7687 --max-connections 200
```

With the default settings, you'll see log lines like the following (structured JSON logs are printed alongside them). Authentication is disabled by default; see [Authentication](./operations/auth.md) for production settings.

```text
Starting with empty database (will save to maharit.db)
HTTP monitoring server listening on 127.0.0.1:9090
Server listening on 127.0.0.1:7687
WARN: maharit-server authentication is DISABLED. All requests are accepted without sessionToken.
```

When stopped with `Ctrl+C` (SIGINT) or SIGTERM, the server saves the data to the file before exiting.

## 5. Connect from a client

### Rust client

Add the dependencies to `Cargo.toml` (replace `/path/to/maharit-db` with the path of your clone).

```toml
[dependencies]
maharit-client = { path = "/path/to/maharit-db/crates/maharit-client" }
tokio = { version = "1", features = ["full"] }
```

Basic usage:

```rust
use maharit_client::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to the server
    let mut client = Client::connect("127.0.0.1:7687").await?;

    // Create a node
    client.execute("CREATE (n:Person {name: \"Charlie\", age: 35})").await?;

    // Run a query and receive the results
    let result = client.query("MATCH (n:Person) RETURN n.name, n.age").await?;
    for row in &result.rows {
        println!("{:?}", row);
    }

    Ok(())
}
```

!!! warning "Query parameters"
    The Cypher `$param` syntax works in the embedded executor (`maharit-query`), but the current TCP protocol and clients cannot send parameters. When you include user input in a query, escape `"` and `\` in string literals before building the query.

## 6. Visualize with the web UI (`maharit-viz`)

`maharit-viz` is an HTTP server that acts as a frontend for `maharit-server`.
It lets you run queries and view the results as a table or a graph in your browser.

```bash
# Start the server (port 7687)
./target/release/maharit server --data /tmp/maharit.db

# In another terminal, start maharit-viz (port 8080)
cargo run --release -p maharit-viz -- \
  --bind 127.0.0.1:8080 \
  --server 127.0.0.1:7687
```

Open <http://localhost:8080> in your browser.

- **Table** tab: filtering, sorting, and pagination with Tabulator
- **Graph** tab: network graph view with cytoscape.js
  (shown automatically when the query returns `<prefix>.id` columns)
- **Raw JSON** tab: the raw JSON of the API response

See the [web app documentation](./visualization/web-app.md) for details.

## 7. Run with Docker

To use Docker, you need to install Docker separately.

```bash
# Build the image
docker build -t maharit-db .

# Start in REPL mode
docker run -it maharit-db

# Start in server mode (publish the port)
docker run -p 7687:7687 maharit-db server --host 0.0.0.0 --port 7687

# Start the server with persistent data
docker run -v maharit-data:/data -p 7687:7687 maharit-db server --host 0.0.0.0 --data /data/maharit.db
```

### Run the server and web UI together with docker-compose

The repository's `docker-compose.yml` defines the `maharit-server` (port 7687) and
`maharit-viz` (port 8080) services.

```bash
docker compose up -d

# Open http://localhost:8080 in your browser
```

## Next steps

- Understand the crate layout in the [architecture overview](./architecture.md)
- Learn the query language in [Cypher basic syntax](./cypher/basics.md)
- Check the available functions in the [function reference](./functions/string.md)
- Configure for production in [server configuration](./operations/server-config.md)
- See how to visualize from the browser in the [web app](./visualization/web-app.md) docs
