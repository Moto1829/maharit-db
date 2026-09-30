# タスク 116: レプリケーションの WAL 差分が SET/REMOVE を送らず、書き込み毎に全件スキャンする

## 概要

リーダーは書き込みクエリの前後でノード/エッジ ID 集合を比較し、その差分を WAL としてフォロワーへ送っている
（`crates/maharit-server/src/tcp_server.rs` の `emit_wal_diff`、呼び出し元は :1113 付近ほか 3 箇所）。

```rust
// 実行前: 全 ID を HashSet に収集
let (node_ids_before, edge_ids_before) = if is_write && replication.is_some() {
    (graph.node_ids().into_iter().collect(), graph.edge_ids().into_iter().collect())
} ...
// 実行後: all_nodes()/all_edges() を走査して「新規 ID」「消えた ID」だけを送る
emit_wal_diff(graph.as_ref(), &node_ids_before, &edge_ids_before, repl).await;
```

## 問題

### 1. 正しさ: 既存要素の変更がフォロワーへ複製されない
`emit_wal_diff` が出すのは CreateNode / DeleteNode / CreateEdge / DeleteEdge と、**新規要素の** SetProperty のみ。
以下はフォロワーに一切反映されない（リーダーとフォロワーのデータが食い違う）:

- 既存ノード/エッジへの `SET n.prop = …`（`MATCH … SET`、`MERGE … ON MATCH SET` 等）
- `REMOVE n.prop` / `REMOVE n:Label`、`SET n:Label`（ラベル追加）
- `SET n = {…}` / `SET n += {…}`

`scripts/replication_test.py` / `failover_test.py` には SET/REMOVE のケースがなく、E2E でも検出されていない。
フェイルオーバー後に昇格したフォロワーは古い値を返す。

### 2. 正しさ: 並行書き込みの差分が混ざる
ConcurrentGraph はロックなしで並行書き込みされるため、クエリ A の before/after の間に
クエリ B が作ったノードも A の差分として送られる（B 自身も送るため重複送信になりうる）。
フォロワー側の適用が冪等でない場合は ID 衝突やエラーの原因になる。

### 3. 性能: 書き込み毎に O(N) のスキャンとアロケーション
レプリケーション有効時、1 件の CREATE でも全ノード・全エッジの ID を `HashSet` に集め、
実行後に `all_nodes()` / `all_edges()` を全走査している。グラフが大きいほど書き込みスループットが線形に劣化する。

## 修正方針（案）

- Executor に「変更ログ（mutation log）」を持たせ、CreateNode / DeleteNode / SetProperty / RemoveProperty /
  AddLabel / RemoveLabel / CreateEdge / DeleteEdge を実行時に記録する
- `into_managers` と同様に実行後に mutation log を取り出し、そのまま `WalEntryData` に変換して送る
  - 必要なら `WalEntryData` に `RemoveProperty` / `AddLabel` / `RemoveLabel` を追加し、フォロワー側の適用処理も実装
- `take_concurrent_snapshot` / `record_undo_diff_concurrent`（tx の undo 用）も同じ mutation log で置き換えられるか検討
- ID 集合スナップショットと `emit_wal_diff` は削除

## 受け入れ条件

- [ ] 既存ノード/エッジへの SET・REMOVE・ラベル変更がフォロワーに反映されるユニットテスト
- [ ] 並行書き込み時に WAL エントリが重複しないこと
- [ ] `scripts/replication_test.py` に SET / REMOVE / ラベル変更のケースを追加しグリーン
- [ ] `scripts/failover_test.py` グリーン
- [ ] レプリケーション有効時の書き込みベンチがノード数に依存しないこと

## 対象ファイル

- `crates/maharit-server/src/tcp_server.rs`（`emit_wal_diff` と呼び出し 3 箇所）
- `crates/maharit-server/src/replication.rs`（`WalEntryData`、フォロワー適用処理）
- `crates/maharit-query/src/executor.rs`（mutation log 記録）
- `scripts/replication_test.py`
