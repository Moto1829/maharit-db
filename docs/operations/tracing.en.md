---
title: Logging
parent: Server and operations
nav_order: 6
source_commit: ed074894
---

# Logging

`maharit server` writes what it does as structured JSON logs, one per line, to **standard output**. Some human-oriented messages, such as the startup message, go to the same standard output.

!!! note "OpenTelemetry"
    Sending traces to OpenTelemetry (OTLP) is not supported yet. The time each query takes is recorded in the `query completed` log entries below.

## Example output

```text
{"timestamp":"2026-11-11T22:53:30.745852Z","level":"INFO","fields":{"message":"Server listening","address":"127.0.0.1:7687","transport":"plaintext"},"target":"maharit::tcp_server","filename":"crates/maharit-server/src/tcp_server.rs","line_number":1087}
{"timestamp":"2026-11-11T22:53:31.915345Z","level":"INFO","fields":{"message":"Client connected","peer":"127.0.0.1:54300"},"target":"maharit::tcp_server","filename":"crates/maharit-server/src/tcp_server.rs","line_number":1167}
{"timestamp":"2026-11-11T22:53:31.915482Z","level":"INFO","fields":{"message":"query completed","duration_us":18},"target":"maharit::tcp_server","filename":"crates/maharit-server/src/tcp_server.rs","line_number":1441,"span":{"query":"CREATE (:A)","name":"query"},"spans":[{"query":"CREATE (:A)","name":"query"}]}
{"timestamp":"2026-11-11T22:53:31.915741Z","level":"INFO","fields":{"message":"Client disconnected","peer":"127.0.0.1:54300"},"target":"maharit::tcp_server","filename":"crates/maharit-server/src/tcp_server.rs","line_number":1230}
```

| Field | Content |
|---|---|
| `timestamp` | Time in UTC |
| `level` | `ERROR` / `WARN` / `INFO` / `DEBUG` / `TRACE` |
| `fields.message` | What happened |
| `fields.*` | Event-specific values (`peer`, `duration_us`, and so on) |
| `span` / `spans` | The operation in progress. For queries, `span.query` holds the query text |
| `target` / `filename` / `line_number` | The module and source location that wrote the entry |

## Main events

| `message` | Level | Values |
|---|---|---|
| `Server listening` | INFO | `address`, `transport` (`plaintext` / `tls`) |
| `Client connected` / `Client disconnected` | INFO | `peer` |
| `Connection error` | WARN | `peer`, `error` |
| `query completed` | INFO | `duration_us` (microseconds), `span.query` |
| `streaming query` | INFO | `query` |
| `checkpoint completed` | INFO | |
| `promoted to leader` | WARN | `replication_bind` |

At startup, a WARN is logged when authentication is off or when listening on a network in plaintext.

!!! warning "Query text is logged"
    `query completed` includes the query text as is. Personal data written directly into a query ends up in the logs too, so pass values as parameters (`$name`); parameter values are not logged. `CREATE USER` / `ALTER USER` statements are not logged.

## Changing the log level

Set the level with the `RUST_LOG` environment variable (default `info`).

```bash
# Warnings and errors only
RUST_LOG=warn maharit server

# Stop the query logs (no INFO from tcp_server)
RUST_LOG=info,maharit::tcp_server=warn maharit server
```

## Collecting logs

Send standard output to the systemd journal, a Docker logging driver, Fluent Bit / Vector, and so on. One JSON object per line can be ingested as is.

```bash
maharit server --data /var/lib/maharit/maharit.db >> /var/log/maharit/server.log
```

Non-JSON lines (human-oriented messages such as `Server listening on …`) are mixed in, so have the collector drop lines that are not valid JSON or keep them as plain strings.
