# タスク 167: EXPLAIN がインデックスを反映せず、常に NodeByLabelScan と表示する

## 現象（2026-11-12、task139 で property-index.md の出力例を確認）
`maharit server` で `CREATE INDEX ON :Person(name)` の後に
`EXPLAIN MATCH (n:Person {name: "p1"}) RETURN n.name` を実行しても、`IndexSeek` ではなく
`NodeByLabelScan` と表示される（範囲条件でも `IndexRangeScan` にならない）。Est. Rows も 20 件中 2 と実態に合わない。
実行自体はインデックスを使っている可能性があり、EXPLAIN の計画（planner に渡す `GraphStats` の
`indexed_properties` やラベル件数）が実際のインデックスを知らないのが原因と思われる。

## 作業
- [ ] サーバー・REPL の EXPLAIN / PROFILE で、実際のインデックスと件数から `GraphStats` を作る
- [ ] docs/indexes/property-index.md・docs/advanced/query-optimization.md の出力例と照合
