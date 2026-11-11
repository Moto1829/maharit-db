# タスク 164: リレーションシップの型を複数指定できるようにする `[:ROAD|FERRY]`

## 現状（2026-11-12）
`MATCH p = (a)-[:ROAD|FERRY*1..5]->(b)` がパースできない（`EdgePattern.edge_type` が `Option<String>`）。
docs/algorithms/all-paths.md には「未対応」と書いてある（実装したら例を戻す）。

## 作業
- [ ] `edge_type` を型のリストにし、1 ホップ・可変長・shortestPath の照合で「いずれかに一致」にする
- [ ] CREATE / MERGE では型を 1 つに限る（複数ならエラー）
- [ ] EXPLAIN の表示、`type(r)`、テスト
