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

- [ ] BEGIN → CREATE/SET → ROLLBACK 後、フォロワーのデータがリーダーと一致するテスト
- [ ] ROLLBACK 後にプロパティインデックス経由の検索結果がグラフと一致
- [ ] tx 内の書き込みでグラフ全体のスナップショットを取らない
