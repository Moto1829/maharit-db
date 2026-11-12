---
title: メトリクス・ヘルスチェック
parent: サーバー・運用
nav_order: 5
---

# メトリクス・ヘルスチェック

`maharit server` は、クライアント用のポートとは別に、監視用の HTTP サーバーを起動します。Prometheus 形式のメトリクスと、ヘルスチェックのエンドポイントを返します。

## 監視用 HTTP サーバー

```bash
maharit server --metrics-bind 0.0.0.0:9090
```

- `--metrics-bind <ADDR>`: 監視用 HTTP サーバーの待ち受けアドレス（既定: `127.0.0.1:9090`。既定ではループバックだけ）
- TLS と認証はありません。ネットワークに公開するときは、ファイアウォールなどで Prometheus からだけ届くようにしてください

| パス | 内容 |
|---|---|
| `/metrics` | Prometheus 形式のメトリクス |
| `/health` | 全体の状態（JSON） |
| `/health/live` | Liveness プローブ用 |
| `/health/ready` | Readiness プローブ用 |

それ以外のパスは `404 Not Found` を返します。

## メトリクス（`/metrics`）

```bash
curl http://127.0.0.1:9090/metrics
```

| メトリクス | 種類 | 説明 |
|---|---|---|
| `maharit_queries_total{type}` | counter | 実行したクエリ数。`type` は `create` / `match` / `delete` / `set` / `merge` / `remove` / `unwind` / `constraint` / `other` |
| `maharit_errors_total` | counter | エラーになったクエリ数 |
| `maharit_query_latency_microseconds{quantile}` | summary | クエリのレイテンシ（マイクロ秒）。`quantile` は `0.5` / `0.95` / `0.99` |
| `maharit_nodes_total` | gauge | ノード数 |
| `maharit_edges_total` | gauge | リレーションシップ数 |
| `maharit_connections_current` | gauge | 現在の接続数 |
| `maharit_connections_total` | counter | 累計の接続数 |
| `maharit_memory_usage_bytes` | gauge | プロセスのメモリ使用量（RSS） |
| `maharit_uptime_seconds` | gauge | 起動からの秒数 |

出力の一部:

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

## ヘルスチェック

### /health

```bash
curl http://127.0.0.1:9090/health
```

```json
{"status":"healthy","live":true,"ready":true}
```

準備ができていないときは HTTP 503 で `"status"` が `"degraded"` または `"unhealthy"` になります。

### /health/live

プロセスが応答できれば HTTP 200 を返します。

```json
{"live":true}
```

### /health/ready

リクエストを受け付けられるときは HTTP 200、そうでなければ HTTP 503 を返します。

```json
{"ready":true}
```

監視用 HTTP サーバーはデータファイルの読み込みと WAL の再適用が終わってから起動するため、応答が返った時点でクライアント用のポートも使えます。

## Prometheus の設定

```yaml
scrape_configs:
  - job_name: 'maharit-db'
    static_configs:
      - targets: ['maharit.example.com:9090']
    scrape_interval: 15s
```

## Kubernetes のプローブ

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

Pod の外からプローブするので、`--metrics-bind 0.0.0.0:9090` で起動してください。
