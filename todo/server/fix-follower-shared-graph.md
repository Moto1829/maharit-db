# Fix: フォロワーノードのレプリケーションデータが TcpServer から見えない問題

## 問題

フォロワーノードが `FollowerReplicationManager::new()` を使って独立した `Graph` オブジェクトを作成していたため、WAL エントリが適用されるグラフと TcpServer がクエリを実行するグラフが別のオブジェクトになっていた。

## 修正内容

### 1. `maharit-core/src/concurrent_graph.rs`

`ConcurrentGraph` に `create_node_with_id_and_labels(id, labels)` メソッドを追加。WAL リプレイ時に特定 ID でノードを再作成し、`next_node_id` カウンタを CAS ループで安全に更新する。

### 2. `maharit-server/src/replication.rs`

- `FollowerReplicationManager.graph` フィールドの型を `Arc<RwLock<Graph>>` → `Arc<ConcurrentGraph>` に変更
- `new()` で `ConcurrentGraph::new()` を使用するように変更
- `with_graph()` を `with_concurrent_graph()` にリネームし、型も変更
- `graph()` ゲッターの戻り値型を `Arc<ConcurrentGraph>` に変更
- `apply_wal_entry()` を `async fn` から同期 `fn` に変更（`ConcurrentGraph` は非同期ロック不要）
- `run_follower_receive_loop()` の `graph` 引数型を `Arc<ConcurrentGraph>` に変更
- テストコードを新しいインタフェースに合わせて更新

### 3. `maharit-server/src/main.rs`

フォロワーモードで `FollowerReplicationManager::with_concurrent_graph(repl_config, Arc::clone(&graph_arc))` を呼び出すよう変更し、TcpServer と同じ `graph_arc` を共有するようにした。

## 状態

- [x] 実装完了
- [x] ビルド成功
- [x] テスト 189 件全通過
