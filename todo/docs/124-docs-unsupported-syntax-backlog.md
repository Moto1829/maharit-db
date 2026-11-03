# タスク 124: docs に残っている未対応構文の解消

## 概要

task 123 で docs の ```cypher ブロックをパース検査するテスト
（`crates/maharit-query/tests/docs_cypher_examples.rs`）を追加した。
既存の記述でパースできないものは `crates/maharit-query/tests/docs_cypher_known_failures.txt`
に一覧化してある（2026-11-03 時点で 64 件）。各項目について「エンジンに実装する」か
「docs を実装に合わせて直す」かを決めて消化し、一覧から外す。

## 分類（2026-11-03 時点）

### エンジン側の未対応（実装候補）
- 句の連結: `UNWIND … MATCH`、`UNWIND … UNWIND`、`MATCH … UNWIND`、トップレベルの `MATCH … MATCH`、
  `MATCH … CREATE … SET`、`CREATE … CREATE`（unwind-with-union / aggregation / string / transactions / query-optimization）
- 式: `%` 演算子、リテラルリストの添字・スライス（`[1,2,3][0]`、`[..3]`）、リスト内包の `WHERE` のみ形、
  `SKIP $a * ($b - 1)` のような式（list / predicates / order-limit-skip）
- 関数: `sin` / `asin` / `atan2` / `exp` / `pow`、`length("str")`、`startsWith()` などの関数形、
  `exists(n.prop)`（math / string / predicates）
- パターン: 複数リレーションタイプ `[:ROAD|FERRY*]`（all-paths）、`CREATE (n:Person $props)`（parameters）、
  `SET n:A:B` の複数ラベル、`SET n = {…}`（set-remove-merge）、`CREATE INDEX FOR (n:L) ON (n.p)`（query-optimization）
- 予約語: 変数名 `end`（predicates。CASE の END と衝突）

### サーバー/プロトコル側の機能（Cypher としては docs を直す候補）
- `GRANT` / `REVOKE` / `DENY` / `SHOW ACL` / `SET DEFAULT ACL`（acl.md）
- `CREATE USER … WITH PASSWORD … ROLE …`、`LOGOUT`（auth.md。実際の構文は `CREATE USER x SET PASSWORD …`）
- `BEGIN` / `COMMIT`（transactions.md。トランザクションは TCP プロトコルの `begin` リクエストで行う）
- `CALL { … }` 以外の形のエクスポート（export.md）

## 完了条件
- `docs_cypher_known_failures.txt` が空（コメント行のみ）になる
