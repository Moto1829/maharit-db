---
title: Web app
parent: Visualization
nav_order: 0
source_commit: 2985da44
---

# Web app (`maharit-viz`)

The `maharit-viz` binary is an HTTP server for running queries against MaharitDB from the browser and visualizing the results as tables and graphs.
It runs as a front end to `maharit-server` (TCP), in a separate process (or container).

```
Browser → maharit-viz (HTTP:8080) → maharit-server (TCP:7687)
              ↕
        Static assets (HTML/JS)
```

## Features

- **Query editor**: a textarea and a run button (run with `Ctrl/⌘+Enter`)
- **Table tab**: table view with Tabulator 6.3
  - Per-column filtering and sorting
  - Pagination by 25 / 50 / 100 / 250 rows
  - Reorder columns by dragging
- **Graph tab**: graph view with cytoscape.js 3.30
  - Detects `<prefix>.id` columns automatically and groups them into nodes
  - Creates an edge when one row has several node groups
  - Label candidates: `.name` / `.title` / `.label`
  - cose layout with Bézier-curve edges
- **Raw JSON tab**: shows the API response as is

## Starting it

### Running locally

Build the binaries first with `cargo build --release -p maharit-server -p maharit-viz`.

```bash
# Start maharit-server (port 7687)
./target/release/maharit server --data /tmp/maharit.db

# Start maharit-viz in another terminal (port 8080)
./target/release/maharit-viz \
  --bind 0.0.0.0:8080 \
  --server 127.0.0.1:7687
```

Open <http://localhost:8080> in the browser.

### CLI options

| Option | Default | Description |
|-----------|--------|-----|
| `--bind`, `-b` | `0.0.0.0:8080` | Bind address of the HTTP listener |
| `--server`, `-s` | `127.0.0.1:7687` | TCP address of the `maharit-server` to connect to |
| `--assets`, `-a` | Bundled with the crate | Directory to serve static assets from |
| `--help`, `-h` | — | Show help |

### Running with Docker

The repository's `docker-compose.yml` defines a `maharit-viz` service.

```bash
docker compose up -d
```

| Service | Host port | Purpose |
|---------|-------------|------|
| `maharit-server` | `7687` (TCP) / `9090` (HTTP) | The database / metrics |
| `maharit-viz` | `8080` (HTTP) | The web app |

`maharit-viz` waits for the server to finish starting via `depends_on.maharit-server.condition: service_healthy`.

## HTTP API

`maharit-viz` itself provides this REST API (also used by the front end).

### `POST /api/query`

Runs a query and returns the results.

```http
POST /api/query
Content-Type: application/json

{ "query": "MATCH (n:Person) RETURN n.id, n.name" }
```

Response on success:

```json
{
  "columns": ["n.id", "n.name"],
  "rows": [
    { "n.id": "\"alice\"", "n.name": "\"Alice\"" },
    { "n.id": "\"bob\"",   "n.name": "\"Bob\"" }
  ],
  "elapsed_ms": 3
}
```

Response on error (HTTP 400):

```json
{ "error": "backend error: server error: Parse error: ..." }
```

`columns` is the set of keys across all rows, sorted in ascending order; missing columns in a row are filled with `null`.

### `GET /api/info`

Returns the server address and the viz version.

```json
{ "server_addr": "127.0.0.1:7687", "version": "0.2.0" }
```

### `GET /api/health`

Health-check endpoint. Returns just the string `ok`.

### `GET /` (and any static path)

Serves the directory given with `--assets` with `tower-http`'s `ServeDir`.
The root returns `index.html`.

## Query patterns for the graph view

The Graph tab extracts nodes by detecting `<prefix>.id` columns in the query results.
To visualize a graph, write a query that returns node IDs and both ends of each relationship.

### Drawing relationships between two nodes

```cypher
MATCH (a)-[r]->(b)
RETURN a.id, a.name, b.id, b.name
LIMIT 100
```

- `a.id` and `b.id` are detected and become node groups `a` and `b`
- Both groups appear in the same row, so an edge `a → b` is drawn
- The `.name` columns are used as node labels

### Showing nodes only

```cypher
MATCH (n:Person)
RETURN n.id, n.name
LIMIT 50
```

- Only node group `n` is detected, and a graph without edges is drawn

### Notes

- Without an `n.id` column, the Graph tab explains that the graph view needs a query that includes `<prefix>.id`.
- The current detection is a simple heuristic. For complex graph visualization, combine it with [DOT / SVG export](export.md) and use Graphviz or other external tools.

## Architecture

- Back end: `axum` 0.7 (Rust 2024 edition)
- Front end: vanilla JS + Tabulator + cytoscape.js (all loaded from CDNs)
- Communication: connects with the `maharit-client` crate over the `maharit-server` TCP protocol (4-byte length prefix + JSON)
- Static assets: served from an external directory with `tower-http::services::ServeDir`

Source code:

- `crates/maharit-viz/src/web.rs` — the axum server and API handlers
- `crates/maharit-viz/src/bin/viz.rs` — the CLI entry point
- `crates/maharit-viz/assets/index.html` — the front-end SPA
