# タスク 114: 共有インデックス/制約マネージャのクエリ毎クローンと lost update

## 概要

TCP サーバーはクエリ実行のたびに `ConstraintManager` / `FulltextManager` / `PropertyIndex` を
Mutex から **deep clone** して Executor に渡し、実行成功後に丸ごと書き戻している。

```rust
// crates/maharit-server/src/tcp_server.rs:1123 (execute_streaming_query)
let cm = constraints.lock().unwrap().clone();
let fm = fulltext.lock().unwrap().clone();
let pi = property_index.lock().unwrap().clone();
let mut executor = unsafe { Executor::new_concurrent_with_managers(graph, cm, fm, pi) };
let result = executor.execute(stmt);
if result.is_ok() {
    let (new_cm, new_fm, new_pi) = executor.into_managers();
    *constraints.lock().unwrap() = new_cm;
    *fulltext.lock().unwrap() = new_fm;
    *property_index.lock().unwrap() = new_pi;
}
```

同じパターンが `execute_query_with_tx`（:1361 付近）と `execute_query`（:1443 付近）にもある。

## 問題

### 1. 正しさ: 並行書き込みで更新が消える（lost update）
clone → execute → 書き戻し の間にロックを保持していないため、並行する 2 つの書き込みクエリ A, B で

1. A が pi を clone
2. B が pi を clone
3. A が node a を作成し、pi_A に a を追加して書き戻す
4. B が node b を作成し、pi_B（a を含まない）で **上書き**

→ グラフには a が存在するのに PropertyIndex から a が消える。
インデックス経由の MATCH（WHERE 等価/範囲プッシュダウン）で a がヒットしなくなる。
FulltextManager（全文検索）や ConstraintManager（CREATE CONSTRAINT の登録そのもの）でも同様。
UNIQUE 制約チェックも各クエリが古いスナップショットを見るため、並行 CREATE で重複をすり抜ける。

### 2. 性能: 読み取りクエリでも O(インデックスサイズ) のコピー
`FulltextManager`（転置インデックス）と `PropertyIndex`（exact/range/reverse index）は
データ量に比例して大きくなる。読み取り専用クエリでも毎回 3 つ clone しており、
大規模 DB ではクエリ毎のレイテンシとアロケーションが支配的になりうる。

## 修正方針（案）

- **最小修正**: 
  - `is_read_only(&stmt)` のときは clone せず、ロック（または `RwLock` の read）を借用して実行する
  - 書き込みクエリは clone〜書き戻しまでを 1 つの write ロック（書き込み直列化用 Mutex）で保護する
- **本修正**: Executor が `Arc<RwLock<…>>`（または `parking_lot`）で共有マネージャを直接参照・更新する形に変更し、
  `new_concurrent_with_managers` / `into_managers` の clone-in/clone-out をなくす
  - 失敗時のロールバックは tx の undo ログ（`record_undo_diff_concurrent`）と整合させる

## 受け入れ条件

- [x] 並行書き込み（例: 8 並列で CREATE を各 1000 件）後、PropertyIndex/Fulltext の件数がグラフと一致するテストを追加
- [x] 並行 CREATE で UNIQUE 制約違反がすり抜けないこと
- [x] 読み取り専用クエリでマネージャの clone が発生しないこと
- [x] `scripts/concurrent_test.py` / `scripts/constraint_test.py` がグリーン
- [ ] `benchmark.py` で読み取りクエリのレイテンシが悪化していないこと（改善が期待される）

## 対象ファイル

- `crates/maharit-server/src/tcp_server.rs`（3 箇所）
- `crates/maharit-query/src/executor.rs`（`new_concurrent_with_managers` / `into_managers`）

## 完了内容 (2026-09-30)

- `Executor` のマネージャ保持を `ManagerSlot`（Owned / Shared / Exclusive）に変更
  - `new_concurrent_shared`（読み取り: 共有参照、clone なし。万一の更新は copy-on-write で私的コピーに閉じる）
  - `new_concurrent_exclusive`（書き込み: 排他参照で共有マネージャを直接更新）
- `tcp_server.rs`: 3 つの `Arc<Mutex<…>>` を `SharedManagers`（3 つの `RwLock` + `write_gate: tokio::sync::Mutex<()>`）に統合
  - 読み取り: read ロックで並行実行
  - 書き込み: `write_gate` で直列化（スナップショット → 実行 → WAL 送出まで保持し、レプリケーション順序も実行順と一致）
  - poison したロックは `into_inner()` で継続利用
  - 失敗した書き込みでもインデックス更新はグラフ変更と同様に残る（以前はグラフだけ変わりインデックスは破棄され不整合だった）
- 回帰テスト追加（multi_thread ランタイム）:
  - `concurrent_writes_keep_property_index_consistent`（8 並列 × 40 CREATE 後に全件が索引経由で引ける）
  - `concurrent_creates_respect_unique_constraint`（16 並列で同一 UNIQUE 値 → 成功は 1 件のみ）
  - 旧実装では両テストとも失敗することを確認（320 件中 200 件超が索引から消失）
- 検証: `cargo test --workspace` 1110 passed / E2E smoke 32, concurrent 19, constraint 26, query_feature 63 全通過
- 未実施: `benchmark.py` による読み取りレイテンシ比較

### 既知の残課題（本タスク範囲外）
- tx の ROLLBACK はグラフのみ undo し、インデックス/制約の変更は巻き戻さない（従来から同じ）
- フォロワーが WAL を適用する際、フォロワー側の PropertyIndex/Fulltext は更新されない
