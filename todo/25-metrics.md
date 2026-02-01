# メトリクス・監視

## 概要
サーバーの状態を監視するためのメトリクス収集・公開機能を実装する。

## 実装内容

### メトリクス収集
- [ ] 接続数（現在/累計）
- [ ] クエリ実行数（種別ごと）
- [ ] クエリレイテンシ（p50, p95, p99）
- [ ] エラー数
- [ ] ノード数/エッジ数
- [ ] メモリ使用量

### Prometheus対応
- [ ] `/metrics` HTTPエンドポイント
- [ ] Prometheus形式でのメトリクス出力
- [ ] カスタムラベル

### OpenTelemetry対応（将来的）
- [ ] トレーシング
- [ ] 分散トレーシング

### ヘルスチェック
- [ ] `/health` エンドポイント
- [ ] Liveness / Readiness 分離
- [ ] カスタムヘルスチェック

### ログ
- [ ] 構造化ログ（JSON形式）
- [ ] ログレベル設定
- [ ] ログローテーション

## 設定
```toml
[metrics]
enabled = true
endpoint = "0.0.0.0:9090"
path = "/metrics"

[health]
enabled = true
endpoint = "0.0.0.0:8080"
```

## 依存クレート候補
- `prometheus` - メトリクス
- `tracing` - 構造化ログ
- `axum` / `warp` - HTTPエンドポイント

## 依存
- `12-tcp-server.md` が完了していること

## 対象クレート
`maharit-server`
