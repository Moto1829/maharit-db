---
title: Metrics and health checks
parent: Server and operations
nav_order: 5
source_commit: ed074894
---

# Metrics and health checks

`maharit server` starts a monitoring HTTP server separate from the client port. It serves Prometheus metrics and health-check endpoints.

## Monitoring HTTP server

```bash
maharit server --metrics-bind 0.0.0.0:9090
```

- `--metrics-bind <ADDR>`: listen address of the monitoring HTTP server (default `127.0.0.1:9090`, loopback only)
- There is no TLS or authentication. If you expose it on a network, make sure only Prometheus can reach it (with a firewall or similar)

| Path | Content |
|---|---|
| `/metrics` | Metrics in Prometheus format |
| `/health` | Overall status (JSON) |
| `/health/live` | For liveness probes |
| `/health/ready` | For readiness probes |

Any other path returns `404 Not Found`.

## Metrics (`/metrics`)

```bash
curl http://127.0.0.1:9090/metrics
```

| Metric | Type | Description |
|---|---|---|
| `maharit_queries_total{type}` | counter | Queries executed. `type` is `create` / `match` / `delete` / `set` / `merge` / `remove` / `unwind` / `constraint` / `other` |
| `maharit_errors_total` | counter | Queries that failed |
| `maharit_query_latency_microseconds{quantile}` | summary | Query latency (microseconds). `quantile` is `0.5` / `0.95` / `0.99` |
| `maharit_nodes_total` | gauge | Number of nodes |
| `maharit_edges_total` | gauge | Number of relationships |
| `maharit_connections_current` | gauge | Current connections |
| `maharit_connections_total` | counter | Total connections |
| `maharit_memory_usage_bytes` | gauge | Process memory usage (RSS) |
| `maharit_uptime_seconds` | gauge | Seconds since startup |

Part of the output:

```text
# HELP maharit_queries_total Total number of queries executed
# TYPE maharit_queries_total counter
maharit_queries_total{type="create"} 0
maharit_queries_total{type="match"} 0
…
# HELP maharit_memory_usage_bytes Current process memory usage (RSS)
# TYPE maharit_memory_usage_bytes gauge
maharit_memory_usage_bytes 4669440
# HELP maharit_uptime_seconds Server uptime in seconds
# TYPE maharit_uptime_seconds gauge
maharit_uptime_seconds 1
```

## Health checks

### /health

```bash
curl http://127.0.0.1:9090/health
```

```json
{"status":"healthy","live":true,"ready":true}
```

When the server is not ready, it returns HTTP 503 with `"status"` set to `"degraded"` or `"unhealthy"`.

### /health/live

Returns HTTP 200 as long as the process can respond.

```json
{"live":true}
```

### /health/ready

Returns HTTP 200 when the server can accept requests, and HTTP 503 otherwise.

```json
{"ready":true}
```

The monitoring HTTP server starts only after the data file has been loaded and the WAL replayed, so once it responds the client port is usable too.

## Prometheus configuration

```yaml
scrape_configs:
  - job_name: 'maharit-db'
    static_configs:
      - targets: ['maharit.example.com:9090']
    scrape_interval: 15s
```

## Kubernetes probes

```yaml
livenessProbe:
  httpGet:
    path: /health/live
    port: 9090
  periodSeconds: 10

readinessProbe:
  httpGet:
    path: /health/ready
    port: 9090
  periodSeconds: 5
```

Probes come from outside the pod, so start the server with `--metrics-bind 0.0.0.0:9090`.
