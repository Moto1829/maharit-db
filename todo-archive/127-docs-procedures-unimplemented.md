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

## 完了（2026-11-14）
- docs 側で解消済み: task140 でアルゴリズムのページを実際の Rust API に合わせて書き直し、centrality.md / connected-components.md に
  「Cypher からは使えません（プロシージャは用意していない）」と明記した。export.md・backup・cache・replication の存在しないプロシージャも
  task124 / task148 で docs から外した。現在 docs に載っているプロシージャは実装済みの `db.index.fulltext.search` だけ（2026-11-14 に grep で確認）
- `relationships(p)` の要素からプロパティを読めない件は bug/169 で修正済み（shortest-path.md の reduce の例も復活）
- プロシージャとして実装する作業は機能追加として query-functions/172 に分けた
