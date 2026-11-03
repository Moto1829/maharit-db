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

## 対応内容（2026-11-03）

| 項目 | 対応 |
|---|---|
| `shortestPath` / `allShortestPaths` | パターン位置で解釈（`PathPattern.shortest: Option<ShortestPathKind>`）。始点ごとに BFS で層を進め、終点パターンに合うノードへの最短経路（All なら同長の全経路）を束縛。最小長は 0/1 のみ、最大長 `*..N` に対応。WHERE は探索後のフィルタとして適用 |
| `YIELD x AS y` と YIELD 後の句 | `ProcedureCallStatement` に `yield_items` / `segments` / `foreach_clause` を追加。YIELD 後に WHERE・MATCH・OPTIONAL MATCH・WITH [WHERE] を続け、末尾は RETURN か FOREACH。存在しない YIELD 列はエラー。FOREACH 付きは `is_read_only` が false |
| `CALL … YIELD … FOREACH (…)` | 上記で対応。あわせて FOREACH のループ変数がノード値なら Node として束縛し `SET n.prop` できるよう修正 |
| `CALL { … UNION [ALL] … }` | `CallSubquery.union` で後続ブランチを連結。列名が一致しないとエラー、UNION は重複除去。トップレベルの `CALL { … } RETURN …`（MATCH なし）にも対応 |
| `CREATE CONSTRAINT ON … ASSERT` | docs を `CREATE CONSTRAINT <name> FOR … REQUIRE …` に書き直し、旧構文は非対応と明記（存在しない自動インデックス・`DROP CONSTRAINT ON …` の記述も修正） |

その他、docs の例を動かすために入れたもの:
- `db.labels()` プロシージャ（conformance の未対応ケースだった `YIELD label AS l` 用）
- `CREATE FULLTEXT INDEX … ON EACH [n.a, n.b]`（Neo4j 形式）
- スカラーとして持ったノード値へのプロパティアクセス（`[n IN nodes(p) | n.name]`）
- lexer の `//` 行コメント・`/* */` ブロックコメント（docs の `--` コメントは `//` に統一）

## 再発防止

- `crates/maharit-query/tests/docs_cypher_examples.rs`: docs 配下の ```cypher ブロックを
  空行区切りのステートメント単位で抽出し、すべてパースできることを検査（`cargo test --workspace` に含まれる）。
- 書式ルール: 1 段落 1 ステートメント、説明・出力例は `//` コメントか ```text ブロック。
  REPL の入出力例やパターン断片は ```text に変更。
- まだパースできない既存の記述は `tests/docs_cypher_known_failures.txt` に列挙（直ったら外すようテストが指示する）。
  消化は task 124 で行う。
- conformance_tests の `KNOWN_UNSUPPORTED` から shortestPath / `YIELD … AS` を `cases()` へ移動。
  旧 `ASSERT` 構文は意図的に非対応のまま残す。

## 検証
- `cargo test --workspace` 全通過、`cargo clippy --workspace --all-targets -- -D warnings` クリーン
- `bash scripts/run_e2e_local.sh`: 全スイート通過

## ステータス
完了
