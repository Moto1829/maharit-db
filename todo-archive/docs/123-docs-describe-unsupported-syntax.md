# タスク 123: ドキュメントに未対応の Cypher 構文が記載されている

## 概要（2026-10-01、docs の cypher コードブロックを EXPLAIN でパース検査して判明）

以下はドキュメントに使い方として載っているが、エンジンは対応していない（以前は後半が黙って無視されて「動いているように見えた」ものもある。bug/118 の厳格化でエラーになる）。

| ドキュメント | 構文 | 対応方針 |
|---|---|---|
| docs/algorithms/shortest-path.md | `MATCH p = shortestPath((a)-[:ROAD*]->(b))` | 実装（パス変数は bug/119 で実装済み、`traversal.rs` に Dijkstra/BFS あり） |
| docs/indexes/fulltext-index.md | `CALL … YIELD node AS article, score WHERE …` | `YIELD x AS y` と YIELD 後の WHERE/MATCH/RETURN を実装 |
| docs/algorithms/connected-components.md | `CALL … YIELD … FOREACH (…)` | YIELD 後の句の連結を実装 |
| docs/cypher/foreach-subquery.md | `CALL { … UNION … }` | CALL サブクエリ内の UNION を実装、または docs 修正 |
| docs/indexes/constraints.md | `CREATE CONSTRAINT ON (n:L) ASSERT …`（Neo4j 3.x 旧構文） | docs を `FOR … REQUIRE` 構文に修正（旧構文は非対応と明記） |

`crates/maharit-query/src/conformance_tests.rs` の `KNOWN_UNSUPPORTED` に一部を登録済み。対応したらそちらから `cases()` へ移す。

## 再発防止
docs の cypher ブロックをパース検査するテスト（または CI ジョブ）を追加する。
コードブロック内の説明文（`結果:` 等）は `// ` コメントにするなど、機械的に抽出できる書式に統一する。
