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

- [ ] 並行書き込み（例: 8 並列で CREATE を各 1000 件）後、PropertyIndex/Fulltext の件数がグラフと一致するテストを追加
- [ ] 並行 CREATE で UNIQUE 制約違反がすり抜けないこと
- [ ] 読み取り専用クエリでマネージャの clone が発生しないこと
- [ ] `scripts/concurrent_test.py` / `scripts/constraint_test.py` がグリーン
- [ ] `benchmark.py` で読み取りクエリのレイテンシが悪化していないこと（改善が期待される）

## 対象ファイル

- `crates/maharit-server/src/tcp_server.rs`（3 箇所）
- `crates/maharit-query/src/executor.rs`（`new_concurrent_with_managers` / `into_managers`）
