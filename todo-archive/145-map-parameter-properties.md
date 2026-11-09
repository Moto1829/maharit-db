# タスク 145: マップのパラメータをプロパティに展開する / パラメータのキーを参照する

## 現状（2026-11-08 確認）
- `CREATE (n:Person $props)` がパースできない（`docs_cypher_known_failures.txt` に登録済み。`docs/cypher/parameters.md` に未対応の注記あり）
- `$props.name`（パラメータのマップのキー参照）もパースできない: `expected }, found .`
- 回避策: `UNWIND [$props] AS p CREATE (n:Person {name: p.name, ...})`

## 作業
- [ ] パーサー: ノード / リレーションシップパターンのプロパティ位置に `$param` を許す（`(n:Label $props)`）。MATCH では等価条件、CREATE / MERGE では全キーを設定
- [ ] `SET n = $props` / `SET n += $props` が動くか確認し、未対応なら対応
- [ ] 式で `$param.key` / `$param['key']` を評価できるようにする
- [ ] known-failures から外し、parameters.md の注記を削除（英語版があれば同時に）
