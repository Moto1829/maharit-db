# タスク: UNIQUE/NOT NULL 制約が CREATE 時に適用されない

## 概要

`CREATE CONSTRAINT ... FOR (n:Label) REQUIRE n.prop IS UNIQUE` や `IS NOT NULL` で制約を作成しても、違反する CREATE クエリがエラーにならずに成功する。また `DROP CONSTRAINT` が "constraint not found" エラーを返し、`SHOW CONSTRAINTS` が 0 件を返すことから、制約がサーバーの ConstraintManager に実際には登録されていない可能性がある。

## 失敗したテスト
- スクリプト: `scripts/constraint_test.py`
- テスト: `UNIQUE 制約` / `NOT NULL 制約` / `SHOW CONSTRAINTS` セクション
- エラーメッセージ:
```
UNIQUE 制約違反でエラーが返る — {'type': 'result', 'rows': [{'created_nodes': '1', 'created_edges': '0'}]}
UNIQUE 重複 CREATE の再確認 — {'type': 'result', 'rows': [{'created_edges': '0', 'created_nodes': '1'}]}
UNIQUE 制約の削除 — {'type': 'error', 'message': 'Execution error: constraint error: constraint not found: unique_product_sku'}
NOT NULL 制約違反（name なし）でエラーが返る — {'type': 'result', 'rows': [{'created_nodes': '1', 'created_edges': '0'}]}
NOT NULL 制約の削除 — {'type': 'error', 'message': 'Execution error: constraint error: constraint not found: notnull_product_name'}
SHOW CONSTRAINTS に作成した制約が含まれる — rows=0
```

## 再現クエリ

```cypher
-- 制約作成（パースは成功するが適用されない）
CREATE CONSTRAINT unique_product_sku FOR (p:Product) REQUIRE p.sku IS UNIQUE
-- 期待: result
-- 実際: result (成功に見える)

-- 重複ノードを作成（制約違反のはず）
CREATE (:Product {sku: 'SKU-001', name: 'Widget'})
CREATE (:Product {sku: 'SKU-001', name: 'Duplicate'})
-- 期待: 2回目はエラー
-- 実際: 両方 result（制約が適用されない）

-- 制約削除
DROP CONSTRAINT unique_product_sku
-- 期待: result
-- 実際: error: constraint not found: unique_product_sku

-- 一覧表示
SHOW CONSTRAINTS
-- 期待: rows に unique_product_sku が含まれる
-- 実際: rows = []（0件）
```

## 根本原因の分析

`CREATE CONSTRAINT` クエリはパースに成功し `result` を返すが、制約が実際に ConstraintManager に登録されていないと考えられる。

考えられる原因:
1. `crates/maharit-query/src/executor.rs` の `execute_create_constraint()` が ConstraintManager への登録を行わずに正常終了している
2. Executor が保持する ConstraintManager のスコープ問題（TCP サーバーが Executor を毎クエリ生成する場合、制約が永続化されない）
3. `executor.rs` の `execute()` 関数が CreateConstraint ケースを未実装または空実装で通過させている
4. WAL/永続化レイヤーへの制約情報の書き込みが行われていない

`DROP CONSTRAINT` が "not found" を返すことは、ConstraintManager が空であることを直接示している。

## 対応方針

1. `crates/maharit-query/src/executor.rs` の `Statement::CreateConstraint` ハンドラを確認
2. TCP サーバーが ConstraintManager をどのように保持・共有しているか調査（`crates/maharit-server/src/tcp_server.rs`）
3. ConstraintManager が Executor と共有されているか、または毎回新規作成されているかを確認
4. 制約の永続化（WAL への書き込み）が実装されているかを確認
5. サーバー起動時の制約ロードが正しく行われているかを確認

## 優先度
HIGH

## 関連ファイル
- `crates/maharit-query/src/executor.rs` — CreateConstraint の実行処理
- `crates/maharit-query/src/ast.rs` — CreateConstraintStatement の定義
- `crates/maharit-server/src/tcp_server.rs` — Executor/ConstraintManager の保持方法
- `scripts/constraint_test.py` — 失敗テスト

## 解決済み (2026-04-14)

**根本原因**: `TcpServer` が `Executor` を毎クエリ新規作成しており、`ConstraintManager` が空のまま生成されていた。

**修正内容**:
- `TcpServer` に `Arc<Mutex<ConstraintManager>>` と `Arc<Mutex<FulltextManager>>` を追加
- `Executor::new_concurrent_with_managers()` および `into_managers()` メソッドを追加
- 全クエリ実行関数（`execute_query`, `execute_query_with_tx`, `execute_streaming_query`）でマネージャーを注入・更新するよう修正
- task_97（DROP FULLTEXT INDEX not found）も同一修正で解決
