# タスク 127: docs に載っているが未実装のプロシージャ・関数

## 概要（2026-11-03、task 123 の作業中に確認）

docs の parse 検査（task 123）は構文だけを見るため、実行時に存在しないプロシージャは検出できない。
現在実装されているプロシージャは `db.index.fulltext.search` と `db.labels` のみ。

## docs に載っている未実装プロシージャ
- `db.connectedComponents` / `db.stronglyConnectedComponents` / `db.labelPropagation`
  （`maharit-core::algorithms` に本体あり。リレーションタイプ指定への対応が必要）
- `db.pageRank` / `db.betweennessCentrality` / `db.closenessCentrality` / `db.centrality.all`
- `db.shortestPath.dijkstra` / `db.allPaths`
- `db.export.dot` / `db.export.subgraph.dot`
- `db.cache.clear` / `db.cache.info` / `db.cache.stats`、`db.backup`、`db.replication.status`

## 関連する式の不足
- `relationships(p)` がエッジ ID（Int）のリストを返すため、`[r IN relationships(p) | r.distance]` や
  `reduce(t = 0, r IN relationships(p) | t + r.distance)` が使えない（shortest-path.md の例）。

## 方針
各プロシージャを実装するか、docs から外して「未実装」と明記する。
実行まで確認するテスト（docs の例をフィクスチャ上で実行する等）も検討する。
