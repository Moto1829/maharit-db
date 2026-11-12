# タスク 156: /metrics のクエリ数・接続数・ノード数が常に 0

## 現象（2026-11-12、task148 の docs 照合で確認）
クエリを実行し接続した後でも、`/metrics` の `maharit_queries_total` / `maharit_errors_total` /
`maharit_query_latency_microseconds` / `maharit_nodes_total` / `maharit_edges_total` /
`maharit_connections_current` / `maharit_connections_total` がすべて 0。
正しいのは `maharit_memory_usage_bytes` と `maharit_uptime_seconds` だけ。

## 原因
`main.rs` の `spawn_http_server` が `Metrics::new()` を新しく作って HTTP サーバーに渡しているが、
`TcpServer` はこの `Metrics` を持たず、どこからも `record_query` / `increment_connections` /
`set_node_count` などを呼んでいない。`/health/ready` も常に ready（起動処理の完了とは連動していない）。

## 作業
- [ ] `TcpServer::with_metrics(Arc<Metrics>)` を追加し、クエリ（種類・所要時間・エラー）と接続数を記録する
- [ ] ノード数・リレーションシップ数はスクレイプ時にグラフから取る
- [ ] コーディネーターも同様に（最低限クエリ数・エラー数）
- [ ] E2E: クエリ後に `/metrics` の値が増えることを確認
- [ ] docs/operations/metrics.md の「既知の不具合」を消す
