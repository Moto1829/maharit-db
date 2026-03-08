---
title: メトリクス・ヘルスチェック
parent: サーバー・運用
nav_order: 5
---

# メトリクス・ヘルスチェック

MaharitDB はサーバーの状態を監視するための Prometheus メトリクスとヘルスチェックエンドポイントを提供します。

## メトリクスエンドポイント

### /metrics

Prometheus 形式のメトリクスを返します。デフォルトポートは `9090` です。

```bash
curl http://localhost:9090/metrics
```

出力例：

```
# HELP maharit_queries_total Total number of queries executed
# TYPE maharit_queries_total counter
maharit_queries_total 12345

# HELP maharit_query_duration_seconds Query execution time in seconds
# TYPE maharit_query_duration_seconds histogram
maharit_query_duration_seconds_bucket{le="0.001"} 9876
maharit_query_duration_seconds_bucket{le="0.01"} 12000
maharit_query_duration_seconds_bucket{le="0.1"} 12300
maharit_query_duration_seconds_bucket{le="1.0"} 12345
maharit_query_duration_seconds_sum 45.67
maharit_query_duration_seconds_count 12345

# HELP maharit_active_connections Current number of active connections
# TYPE maharit_active_connections gauge
maharit_active_connections 42

# HELP maharit_node_count Total number of nodes in the graph
# TYPE maharit_node_count gauge
maharit_node_count 100000

# HELP maharit_edge_count Total number of edges in the graph
# TYPE maharit_edge_count gauge
maharit_edge_count 500000

# HELP maharit_memory_bytes Memory usage in bytes
# TYPE maharit_memory_bytes gauge
maharit_memory_bytes 1073741824

# HELP maharit_uptime_seconds Server uptime in seconds
# TYPE maharit_uptime_seconds gauge
maharit_uptime_seconds 86400
```

## 利用可能なメトリクス

| メトリクス名 | タイプ | 説明 |
|------------|--------|------|
| `maharit_queries_total` | Counter | 実行されたクエリの総数 |
| `maharit_queries_failed_total` | Counter | 失敗したクエリの総数 |
| `maharit_query_duration_seconds` | Histogram | クエリ実行時間 |
| `maharit_active_connections` | Gauge | アクティブな接続数 |
| `maharit_connections_total` | Counter | 累積接続数 |
| `maharit_node_count` | Gauge | グラフ内のノード数 |
| `maharit_edge_count` | Gauge | グラフ内のエッジ数 |
| `maharit_memory_bytes` | Gauge | メモリ使用量（バイト） |
| `maharit_uptime_seconds` | Gauge | サーバー稼働時間 |
| `maharit_wal_size_bytes` | Gauge | WAL ファイルサイズ |
| `maharit_backup_last_timestamp` | Gauge | 最後のバックアップ時刻（Unix 時間） |

## ヘルスチェックエンドポイント

### /health

サーバーの全体的な健全性を返します。

```bash
curl http://localhost:9090/health
```

正常時（HTTP 200）：
```json
{
  "status": "healthy",
  "version": "0.1.0",
  "uptime_seconds": 86400,
  "node_count": 100000,
  "edge_count": 500000,
  "active_connections": 42
}
```

異常時（HTTP 503）：
```json
{
  "status": "unhealthy",
  "reason": "WAL write failed",
  "timestamp": "2024-01-01T00:00:00Z"
}
```

### /health/live

Kubernetes Liveness プローブ用。サーバープロセスが生きているかを確認します。

```bash
curl http://localhost:9090/health/live
```

正常時（HTTP 200）：
```json
{"status": "ok"}
```

### /health/ready

Kubernetes Readiness プローブ用。サーバーがリクエストを受け付けられる状態かを確認します。

```bash
curl http://localhost:9090/health/ready
```

正常時（HTTP 200）：
```json
{"status": "ready"}
```

準備未完了時（HTTP 503）：
```json
{"status": "not_ready", "reason": "initializing"}
```

## Prometheus での設定

`prometheus.yml` にスクレイプ設定を追加します。

```yaml
scrape_configs:
  - job_name: 'maharit-db'
    static_configs:
      - targets: ['localhost:9090']
    scrape_interval: 15s
    metrics_path: /metrics
```

## Grafana ダッシュボード

Prometheus と Grafana を組み合わせてダッシュボードを構築できます。主要なパネル例：

- クエリスループット（QPS）: `rate(maharit_queries_total[1m])`
- クエリレイテンシ（P99）: `histogram_quantile(0.99, rate(maharit_query_duration_seconds_bucket[5m]))`
- アクティブ接続数: `maharit_active_connections`
- グラフサイズ: `maharit_node_count`, `maharit_edge_count`
- メモリ使用量: `maharit_memory_bytes`

## Kubernetes での設定例

```yaml
livenessProbe:
  httpGet:
    path: /health/live
    port: 9090
  initialDelaySeconds: 10
  periodSeconds: 10

readinessProbe:
  httpGet:
    path: /health/ready
    port: 9090
  initialDelaySeconds: 5
  periodSeconds: 5
```
