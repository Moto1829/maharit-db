# タスク 164: リレーションシップの型を複数指定できるようにする `[:ROAD|FERRY]`

## 現状（2026-11-12）
`MATCH p = (a)-[:ROAD|FERRY*1..5]->(b)` がパースできない（`EdgePattern.edge_type` が `Option<String>`）。
docs/algorithms/all-paths.md には「未対応」と書いてある（実装したら例を戻す）。

## 作業
- [x] `edge_type` を型のリストにし、1 ホップ・可変長・shortestPath の照合で「いずれかに一致」にする
- [x] CREATE / MERGE では型を 1 つに限る（複数ならエラー）
- [x] EXPLAIN の表示、`type(r)`、テスト

## 結果（2026-11-14）
- `EdgePattern.edge_types: Vec<String>`。`[:A|B]` と旧来の `[:A|:B]` をパース。`matches_type()` で 1 ホップ・可変長・shortestPath を照合
- CREATE / MERGE は作成前に検査し、型が 2 つ以上なら TypeError（途中までノードを作らない）
- EXPLAIN は `:A|B` と表示。docs（basics / all-paths）に例を追加
