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

- [x] 既存ノード/エッジへの SET・REMOVE・ラベル変更がフォロワーに反映されるユニットテスト
- [x] 並行書き込み時に WAL エントリが重複しないこと
- [x] `scripts/replication_test.py` に SET / REMOVE / ラベル変更のケースを追加しグリーン
- [x] `scripts/failover_test.py` グリーン
- [x] レプリケーション有効時の書き込みベンチがノード数に依存しないこと

## 対象ファイル

- `crates/maharit-server/src/tcp_server.rs`（`emit_wal_diff` と呼び出し 3 箇所）
- `crates/maharit-server/src/replication.rs`（`WalEntryData`、フォロワー適用処理）
- `crates/maharit-query/src/executor.rs`（mutation log 記録）
- `scripts/replication_test.py`

## 完了内容 (2026-09-30)

方針を「Executor に mutation log を持たせる」から **GraphBackend ラッパーで記録する** 形に変更（Executor の書き込みは
すべて `GraphBackend` の 9 メソッドを通るため、Executor 本体の変更が最小で済む）。

- `crates/maharit-server/src/mutation_log.rs`（新規）: `RecordingGraph` が `ConcurrentGraph` を包み、
  CreateNode / CreateEdge / DeleteNode / DeleteEdge / SetProperty / RemoveProperty / AddLabel / RemoveLabel を実行順に記録
  - レプリケーション無効時は記録しない（`enabled=false`）
- `WalEntryData` に `RemoveProperty` / `AddLabel` / `RemoveLabel` を追加、フォロワーの `apply_wal_entry` に適用処理を追加
- `Executor::new_with_backend_exclusive(&mut dyn GraphBackend, …)` を追加（graph ポインタを `*mut (dyn GraphBackend + 'a)` に）
- `emit_wal_diff`（全 ID 集合の前後比較）と書き込み毎の ID スナップショットを削除 → `emit_wal_entries(log)`
- 並行書き込みの差分混入: task114 の `write_gate` で書き込みを直列化し、WAL 送出まで保持するため解消
- 失敗した文でも適用済みの変更は WAL に流す（リーダーのグラフはロールバックされないため）

### 調査中に見つけて併せて修正
- フォロワーが `CreateEdge.edge_id` を無視して独自採番していた → `ConcurrentGraph::create_edge_with_id` を追加して leader の ID を維持
- `Float(1.0)` が `"1"` とエンコードされフォロワーで `Int` になっていた → `{:?}` で `"1.0"` に（snapshot 側のエンコーダも共通化）
- `ConcurrentGraph` に `remove_node_property` / `remove_edge_property`（&self）を追加

### 検証
- ユニットテスト: `RecordingGraph` の記録順・無効時・Float エンコード、`SharedManagers::execute` の SET/REMOVE/ラベル記録、
  フォロワー適用（エッジ ID 維持・RemoveProperty・Add/RemoveLabel・Float 型）
- `cargo test --workspace` 1114 passed、clippy/fmt クリーン
- `replication_test.py` に「既存要素の更新の伝播確認」（SET+ラベル / REMOVE / エッジ SET の Float 型）を追加 → 35/35 通過を確認
- `failover_test.py --no-docker` 18/18
- 単一サーバー E2E（smoke 32 / concurrent 19 / constraint 26 / query_feature 63）全通過

### 残課題（別タスク化）
- replication_test が不安定（変更前バイナリでも 4 回中 3 回失敗 → 既存不具合）: bug/120
- ROLLBACK がフォロワーへ複製されない: bug/117
- tx 経路の undo 用 `take_concurrent_snapshot` は依然 O(N)（`RecordingGraph` のログから undo を作れば解消可能）
- 旧バージョンのフォロワーは新しい WAL バリアントをデシリアライズできない（0.x のためローリング更新非対応として許容）
