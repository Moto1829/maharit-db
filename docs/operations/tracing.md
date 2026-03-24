---
title: OpenTelemetry トレーシング
parent: サーバー・運用
nav_order: 6
---

# OpenTelemetry トレーシング

MaharitDB は OpenTelemetry による分散トレーシングをサポートしています。クエリの実行経路を詳細に追跡し、パフォーマンスのボトルネックを特定できます。

## 概要

OpenTelemetry トレーシングを有効にすると、以下の情報が収集されます。

- クエリの受信から結果返却までの全体的な時間
- パーサー、エグゼキュータ、ストレージへのアクセスの内訳
- エラーの発生場所とエラー詳細
- 接続ごとのトレース ID

## 設定

### OTLP エクスポーターを使用する場合

```bash
maharit server \
  --host 0.0.0.0 \
  --port 7687 \
  --tracing-enabled \
  --tracing-endpoint "http://localhost:4317" \
  --tracing-service-name "maharit-db"
```

環境変数での設定：

```bash
export MAHARIT_TRACING_ENABLED=true
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317
export OTEL_SERVICE_NAME=maharit-db
maharit server
```

### Jaeger での設定

```bash
# Jaeger をバックエンドとして使用
maharit server \
  --tracing-enabled \
  --tracing-endpoint "http://jaeger:4317" \
  --tracing-service-name "maharit-production"
```

### Zipkin での設定

```bash
maharit server \
  --tracing-enabled \
  --tracing-backend zipkin \
  --tracing-endpoint "http://zipkin:9411/api/v2/spans"
```

## スパンの構造

各クエリの実行は以下のスパン階層で記録されます。

```
maharit.query (ルートスパン)
├── maharit.parse       (クエリのパース)
├── maharit.plan        (クエリプランの生成)
├── maharit.execute     (クエリの実行)
│   ├── maharit.graph.match   (グラフマッチング)
│   ├── maharit.graph.create  (ノード/エッジの作成)
│   └── maharit.fulltext      (全文検索)
└── maharit.storage.wal (WAL への書き込み)
```

## スパンに含まれる属性

| 属性名 | 説明 |
|--------|------|
| `db.system` | `maharit` |
| `db.statement` | 実行されたクエリ文字列 |
| `db.operation` | 操作の種類（match, create, etc.） |
| `maharit.node_count` | マッチしたノード数 |
| `maharit.edge_count` | マッチしたエッジ数 |
| `maharit.result_count` | 返却した結果行数 |
| `maharit.duration_ms` | 実行時間（ミリ秒） |
| `net.peer.ip` | クライアントの IP アドレス |
| `error` | エラーが発生した場合は `true` |
| `error.message` | エラーメッセージ |

## サンプリング設定

すべてのリクエストをトレースするとオーバーヘッドが大きくなります。サンプリングレートを設定して負荷を軽減できます。

```bash
# 全トレースを収集（開発環境向け）
export OTEL_TRACES_SAMPLER=always_on

# 10% のトレースのみ収集（本番環境向け）
export OTEL_TRACES_SAMPLER=traceidratio
export OTEL_TRACES_SAMPLER_ARG=0.1

# エラーは必ず収集、正常は 5%
export OTEL_TRACES_SAMPLER=parentbased_traceidratio
export OTEL_TRACES_SAMPLER_ARG=0.05
```

## Jaeger UI での確認

Jaeger を使用している場合、`http://localhost:16686` でトレースを確認できます。

```bash
# Docker で Jaeger を起動
docker run -d \
  -p 16686:16686 \
  -p 4317:4317 \
  jaegertracing/all-in-one:latest

# MaharitDB をトレーシング有効で起動
maharit server \
  --tracing-enabled \
  --tracing-endpoint "http://localhost:4317"

# クエリを実行してトレースを生成
maharit> MATCH (n:Person) RETURN n LIMIT 10
```

## Prometheus との連携

OpenTelemetry のメトリクスを Prometheus に送信することもできます。

```bash
export OTEL_METRICS_EXPORTER=prometheus
export OTEL_EXPORTER_PROMETHEUS_PORT=9464
```

## トレースとログの相関

構造化ログにはトレース ID が含まれます。ログとトレースを相関させることで根本原因分析が容易になります。

```json
{
  "level": "INFO",
  "message": "Query executed",
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
  "span_id": "00f067aa0ba902b7",
  "query": "MATCH (n:Person) RETURN n",
  "duration_ms": 5,
  "result_count": 100
}
```
