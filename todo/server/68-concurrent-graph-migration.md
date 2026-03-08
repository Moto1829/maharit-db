# Task 68: サーバーを ConcurrentGraph に移行する

## 両者の役割

### Graph（graph.rs）
- `Vec<Option<Node>>` による密な配列構造、削除 ID を free-list で再利用
- ロック機構を持たない単一スレッド向けのデータ構造
- 外部で `Arc<RwLock<Graph>>` に包むことでスレッドセーフにする
- Executor・PersistentStorage・TransactionManager がすべて対応済み

### ConcurrentGraph（concurrent_graph.rs）
- `DashMap<NodeId, Node>` によるシャードロック構造
- `&self`（不変参照）で書き込みができる内部可変性を持つ
- 異なるノード/エッジへのアクセスが互いをブロックしない
- ID は単調増加で再利用なし
- Executor・PersistentStorage 等は**未対応**

## 背景・目的

現在のサーバーは本来 `ConcurrentGraph` を使うべきだが、
Executor 等が `Graph` に依存しているため `Arc<RwLock<Graph>>` で代替している。

`Arc<RwLock<Graph>>` の問題:
- 書き込みクエリが来ると読み取りも含めた全クエリがブロックされる
- グラフ全体に1つのロックがかかるため並行性が低い

`ConcurrentGraph` に移行することで:
- 書き込み中でも読み取りを並行実行できる
- ノード/エッジ単位のシャードロックで書き込み競合も局所化される

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
- [x] Phase 1: Executor 対応（GraphBackend トレイト, `*mut dyn GraphBackend`, `new_concurrent()`）
- [x] Phase 2: PersistentStorage 対応（`save_concurrent` / `load_concurrent`）
- [x] Phase 3: TcpServer 切り替え（`Arc<ConcurrentGraph>`, ロックレスクエリ実行）
- [ ] Phase 4: TransactionManager 対応（オプション）
