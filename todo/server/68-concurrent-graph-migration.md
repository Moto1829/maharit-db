# Task 68: サーバーを ConcurrentGraph に移行する

## 背景

現在のサーバーは `Arc<RwLock<Graph>>` を使っており、書き込み中は全クエリがブロックされる。
`ConcurrentGraph`（DashMap ベース）に移行することで書き込み中でも読み取りを並行実行できるようになる。

## 段階的移行計画

### Phase 1: Executor を ConcurrentGraph に対応させる
- Executor が現在 `&mut Graph` を要求している
- ConcurrentGraph 用の ExecutorConcurrent または Executor のジェネリック化
- 既存の Graph 向け Executor は維持し、新実装と並存させる

### Phase 2: PersistentStorage を ConcurrentGraph に対応させる
- `PersistentStorage::load_concurrent()` -> ConcurrentGraph
- `PersistentStorage::save_concurrent(&ConcurrentGraph, path)`
- 既存の Graph 用 save/load は維持する

### Phase 3: TcpServer を ConcurrentGraph に切り替える
- TcpServer の graph フィールドを `Arc<ConcurrentGraph>` に変更
- `Arc<RwLock<Graph>>` を廃止
- GraphSnapshot / record_undo_diff も ConcurrentGraph 対応

### Phase 4: トランザクション対応（オプション）
- TransactionManager の rollback() が `&mut Graph` を要求している
- ConcurrentGraph 向けの rollback 実装

## 注意事項
- 各 Phase は独立してビルド・テストできるようにする
- Phase 1 完了後に Phase 2、という順序を守る
- 既存テストが壊れないことを各 Phase で確認する

## ステータス
- [ ] Phase 1: Executor 対応
- [ ] Phase 2: PersistentStorage 対応
- [ ] Phase 3: TcpServer 切り替え
- [ ] Phase 4: TransactionManager 対応（オプション）
