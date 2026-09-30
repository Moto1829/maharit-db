# タスク 117: トランザクション ROLLBACK がフォロワーへ複製されない

## 概要

`Request::Rollback` は `tx_manager.rollback_concurrent(tx_id, &graph)` でリーダーのグラフだけを undo する
（`crates/maharit-server/src/tcp_server.rs` の Rollback ハンドラ）。tx 内の書き込みは実行時に WAL として
フォロワーへ送られている（task116）ため、ROLLBACK 後はリーダーとフォロワーのデータが食い違う。

また ROLLBACK は `SharedManagers::write_gate` を取得しておらず、他の書き込みと WAL 順序が保証されない。
undo はインデックス（PropertyIndex / Fulltext）も巻き戻さない。

## 修正方針（案）

- Rollback を `write_gate` 下で実行し、undo 操作を `RecordingGraph` 経由で適用して WAL に流す
  （`rollback_concurrent` を `&mut dyn GraphBackend` 受けに一般化）
- undo 時の DeleteNode 復元が新 ID で再作成している点（`create_node_with_labels`）も、ID 維持に変更を検討
- tx 経路の undo 用 `take_concurrent_snapshot`（O(N)）を `RecordingGraph` のログから undo を作る方式に置き換え

## 受け入れ条件

- [x] BEGIN → CREATE/SET → ROLLBACK 後、フォロワーのデータがリーダーと一致するテスト
- [x] ROLLBACK 後にプロパティインデックス経由の検索結果がグラフと一致
- [x] tx 内の書き込みでグラフ全体のスナップショットを取らない

## 完了内容 (2026-09-30)

### 修正
- **undo の生成を実行時記録に統一**: `RecordingGraph::with_undo()` が書き込みごとに旧値を読んで `UndoRecord` を生成。
  tx の書き込み毎に取っていた全グラフスナップショット（`take_concurrent_snapshot`）と差分計算（`record_undo_diff_concurrent`）を削除（O(N) → O(書き込み量)）
- **ROLLBACK の複製**: `TransactionManager::rollback_backend(&mut dyn GraphBackend)` で undo を適用。サーバーは `RecordingGraph` 越しに適用するため、
  補償操作がそのまま WAL としてフォロワーへ流れる。`write_gate` 下で実行し、WAL 順序も保証
- **ID を維持した復元**: `GraphBackend::restore_node` / `restore_edge` を追加（`Graph::create_edge_with_id` 新設）。
  削除ノード/エッジは元の ID で復元され、後続の undo やフォロワーと食い違わない
- **従来 undo されていなかったもの**:
  - ラベル変更（`SET n:L` / `REMOVE n:L`）→ `UndoRecord::AddLabel` / `RemoveLabel` を追加
  - DETACH DELETE で暗黙に消えるエッジ → ノード削除前に接続エッジを undo に記録
  - 途中で失敗した文の部分変更 → 失敗時も undo を記録
- **インデックスの再同期**: ROLLBACK で触れたノードの PropertyIndex / Fulltext を復元後の状態で作り直す
- **終了済み/不明な tx での書き込みを拒否**（従来は実行されて ROLLBACK 不能な変更が残っていた）: `TransactionManager::is_active`
- `rollback(&mut Graph)` も `rollback_backend` に統一、`rollback_concurrent` は削除

### テスト（いずれも修正前のコードで失敗することを確認済み）
- `rollback_restores_graph_labels_edges_and_index`: SET+ラベル / CREATE / DETACH DELETE を ROLLBACK → 索引経由の検索・ラベル・エッジが元通り
- `rollback_is_replicated_to_followers`: 実フォロワー接続で ROLLBACK 後にリーダーとフォロワーのグラフ（ID・ラベル・プロパティ・エッジ）が一致
- `write_with_inactive_tx_is_rejected`
- `undo_capture_round_trips_through_rollback`（mutation_log）: 元 ID での復元、DETACH エッジ復元、補償操作の WAL 出力
- replication_test.py に「ROLLBACK の伝播確認」を追加 → 45/45 を 5 回連続通過、failover 18/18、単一サーバー E2E 全通過、`cargo test --workspace` 1120 passed
