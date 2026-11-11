# タスク 166: REPL でインデックス・制約が次の文に引き継がれない

## 現象（2026-11-12、task139 で property-index.md の例を確認中に発見）
REPL（`maharit` を引数なしで起動）で `CREATE INDEX` / `CREATE CONSTRAINT` / `CREATE FULLTEXT INDEX` が
成功しても、次の文では無かったことになっていた。`SHOW INDEXES` / `SHOW CONSTRAINTS` が空で、
UNIQUE 制約に違反するノードも作れた。REPL が文ごとに新しい `Executor`（空の管理構造）を作っていたため。

## 対応
- [x] REPL が制約・全文検索・プロパティインデックスを持ち、`Executor::new_with_backend_exclusive` で毎回渡す
- [x] テスト（制約違反が拒否され、SHOW INDEXES / SHOW CONSTRAINTS に残る）
