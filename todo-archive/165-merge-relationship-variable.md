# タスク 165: MERGE で作ったリレーションシップの変数を ON CREATE SET / RETURN で参照できない

## 現象（2026-11-12、task139 で set-remove-merge.md の例を実行して発見）
```text
MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"})
MERGE (a)-[r:KNOWS]->(b)
ON CREATE SET r.since = 2024
→ Execution error: undefined variable: r
```
リレーションシップは作成されるが `r` が束縛されず、ON CREATE SET が実行されないままエラーになっていた。
2 回目（一致する側）は ON MATCH SET が効く。

## 対応
- [x] MERGE の作成処理（`create_pattern`）でリレーションシップの変数を束縛する
- [x] テスト
