# タスク 139: docs 英語翻訳 第 2 弾 — Cypher / 関数 / インデックス

前提: タスク 138 完了。進め方はタスク 138 と同じ（`source_commit`・用語集・コードは変えない）。

クエリを書くときに参照されるリファレンス部分。```cypher ブロックが最も多い領域なので、タスク 136 で追加する「英語と日本語の cypher ブロックが同一」テストが特に効く。

## 対象
- [ ] `cypher/where.md`
- [ ] `cypher/set-remove-merge.md`
- [ ] `cypher/unwind-with-union.md`
- [ ] `cypher/foreach-subquery.md`
- [ ] `cypher/order-limit-skip.md`
- [ ] `cypher/parameters.md`
- [ ] `functions/string.md` / `math.md` / `list.md` / `predicates.md` / `aggregation.md` / `fulltext.md` / `temporal.md`
- [ ] `indexes/property-index.md` / `fulltext-index.md` / `constraints.md`

## 注意
- `functions/fulltext.md` と `indexes/fulltext-index.md` の日本語形態素解析（IPADIC）の説明は、英語版でも「日本語テキスト向けの機能」として残す
- タスク 124（docs の未対応構文の解消）で日本語側が書き換わる可能性が高い。先に進んでいる方に合わせ、`source_commit` で追従する

## 完了条件
- 上記の `.en.md` があり、`mkdocs build --strict` と `docs_cypher_examples` テストが通る
