---
title: Architecture overview
nav_order: 3
source_commit: afbcbdf0
---

# Architecture overview

MaharitDB is organized as a Rust workspace made up of several crates, each with a separate responsibility.

## Crates

| Crate | Description |
|---------|------|
| `maharit-core` | Graph data structures, algorithms, full-text search engine |
| `maharit-query` | Cypher query parser, executor, planner |
| `maharit-storage` | WAL, persistence, transactions, backups |
| `maharit-cluster` | Sharding (Hash / Range strategies), routing |
| `maharit-server` | TCP server, authentication, metrics, replication |
| `maharit-client` | Async/sync client, connection pool |
| `maharit-io` | CSV/JSON/GraphML import and export |
| `maharit-viz` | DOT/SVG/ASCII visualization, web app |

## Crate details

### maharit-core

Provides the core graph data structures and operations.

- **Graph struct**: stores nodes (`Node`) and edges (`Edge`) in arrays indexed by ID
- **Traversal**: DFS/BFS, shortest path (Dijkstra), all-paths enumeration
- **Graph algorithms**: PageRank, centrality (betweenness and closeness), connected components
- **Full-text search engine**: inverted index, BM25 scoring, Japanese morphological analysis with lindera
- **GraphStats**: node count, edge count, and per-label statistics

```
maharit-core/src/
├── lib.rs          -- public API exports
├── graph.rs        -- Graph, Node, Edge definitions
├── traversal.rs    -- traversal and path finding
├── algorithms.rs   -- PageRank, centrality, connected components
├── fulltext.rs     -- full-text index (FulltextManager)
├── constraint.rs   -- schema constraints
├── property_index.rs -- property indexes
└── concurrent_graph.rs -- ConcurrentGraph for concurrent reads and writes
```

### maharit-query

Processes the Cypher-like query language.

- **lexer.rs**: lexer (Token, TokenKind)
- **ast.rs**: abstract syntax tree definitions (Statement, Expression, Pattern)
- **parser.rs**: recursive descent parser
- **executor.rs**: query execution engine (Executor)
- **planner.rs**: query plan generation (QueryPlan, PlanNode)

Query processing flow:

```
Input string
  → Lexer (lexical analysis)
  → Parser (syntax analysis)
  → AST (abstract syntax tree)
  → Executor (execution)
    → Graph operations
    → Return results
```

For EXPLAIN/PROFILE, the planner takes part:

```
AST → Planner → QueryPlan → Executor (with measured values)
```

### maharit-storage

Handles data persistence and transaction management.

- **WAL (write-ahead log)**: writes changes to disk ahead of applying them (Rust API; the server's WAL is `durable_wal.rs` in `maharit-server`)
- **Snapshots**: serialize/deserialize the whole graph
- **Transactions**: snapshot isolation with MVCC
- **Backups**: full and incremental backups with gzip compression
- **PITR**: point-in-time recovery using the WAL

### maharit-server

Handles the network layer and server management.

- **TCP server**: tokio async I/O, TLS support (rustls)
- **Authentication and RBAC**: user management, role-based access control
- **Fine-grained ACL**: access control per label and property
- **HTTP server**: `/metrics`, `/health`, `/health/live`, `/health/ready`
- **OpenTelemetry**: distributed tracing
- **WAL**: appends each write query's changes (`WalEntryData`) to `<data>.wal` and replays them at startup; group commit, checkpoints
- **Replication**: WAL streaming, leader/follower setup

### maharit-cluster

Handles sharding across multiple servers.

- **Sharding strategies**: Hash / Range
- **Router**: routes queries to the responsible shard
- **Coordinator**: used from `coordinator.rs` in `maharit-server`

### maharit-client

Provides clients for connecting to the server.

- **Async client**: `Client` (tokio-based)
- **Sync client**: `SyncClient` (wraps the async runtime)
- **Connection pool**: reuses and manages connections
- **Query execution**: `query` (receive results) / `execute` (discard results), and `query_with_params` / `execute_with_params` with parameters

### maharit-io

Exchanges data with external formats.

- **CSV import/export**: bulk input and output of nodes and edges
- **JSON import/export**: JSON Lines format
- **GraphML**: GraphML XML format support

### maharit-viz

Provides graph visualization and a web frontend.

- **DOT output**: export in the Graphviz DOT language
- **SVG export**: SVG generation with a force-directed layout
- **ASCII renderer**: text visualization for the terminal
- **Web app** (`maharit-viz` binary): an axum-based HTTP server
  with browser visualization through Tabulator (tables) and cytoscape.js (graphs)

## Data flow

```
Client (TCP/TLS)
        ↓
    maharit-server
     ├── Authentication and ACL checks
     ├── Receive query
     └── Forward to maharit-query
              ↓
         Executor
          ├── maharit-core (graph operations)
          └── RecordingGraph (records changes as WAL entries, appends them to the WAL file, and sends them to followers)
              ↓
         Return results
              ↑
    maharit-server
        ↑
Client (receives results)
```

## Dependency graph

```
maharit-server
  ├── maharit-query
  │     └── maharit-core
  ├── maharit-storage
  │     └── maharit-core
  └── maharit-cluster
        └── maharit-core

maharit-io
  └── maharit-core

maharit-viz
  ├── maharit-core
  └── maharit-client (web app only)

maharit-client (standalone; does not depend on maharit-core)
```

## Memory model

Graph data is kept in memory. The `Graph` struct stores nodes and edges in arrays indexed by ID:

- `nodes: Vec<Option<Node>>`: node storage (deleted slots are `None`)
- `edges: Vec<Option<Edge>>`: edge storage
- `outgoing_edges: Vec<HashSet<EdgeId>>`: adjacency list of outgoing edges (O(1) deletion)
- `incoming_edges: Vec<HashSet<EdgeId>>`: adjacency list of incoming edges
- `node_free_list` / `edge_free_list`: lists of IDs freed by deletion, for reuse
- `label_index: HashMap<String, HashSet<NodeId>>`: reverse index by label

Node and edge properties are shared as `Arc<HashMap<..>>` to reduce copying when returning results.
The server appends each write query's changes to `<data>.wal` as WAL entries and also sends them from the leader to followers. When it stops (SIGINT / SIGTERM) and when the WAL grows past a set size (a checkpoint), it saves the whole graph to the data file and empties the WAL. Node and edge IDs do not change across save and load (the WAL refers to them by ID).

## Persistence modes

How persistence works depends on which layer you use.

| Layer | Mode | Description |
|----|--------|------|
| Library / API (`maharit-core`) | In-memory | `Graph::new()` needs no file; data is gone when the process exits |
| Server (`maharit-server`) | Always file-backed | Always uses a data file from startup (like SQLite) |

### Using it as a library (in-memory)

When you use `maharit-core` directly, `Graph::new()` gives you an in-memory graph. It suits uses that don't need persistence (tests, temporary graph processing, embedding).

```rust
use maharit_core::Graph;

let mut graph = Graph::new(); // no file needed, in-memory
```

### Using it as a server (always persistent)

Start `maharit-server` with a file path. If the file exists it is loaded; otherwise a new one is created.

```bash
# Start with a file path (default: maharit.db)
maharit server --data /var/lib/maharit/maharit.db

# Can also be set with an environment variable
MAHARIT_DATA=/var/lib/maharit/maharit.db maharit server
```

Writes are recorded in the WAL, so acknowledged changes survive a forced kill. On SIGINT / SIGTERM the server saves the data file and empties the WAL. See [server configuration](operations/server-config.md) for details.
