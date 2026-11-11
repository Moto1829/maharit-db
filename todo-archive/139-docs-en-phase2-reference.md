# タスク 139: docs 英語翻訳 第 2 弾 — Cypher / 関数 / インデックス

前提: タスク 138 完了。進め方はタスク 138 と同じ（`source_commit`・用語集・コードは変えない）。

クエリを書くときに参照されるリファレンス部分。```cypher ブロックが最も多い領域なので、タスク 136 で追加する「英語と日本語の cypher ブロックが同一」テストが特に効く。

## 対象
- [x] `cypher/where.md`
- [x] `cypher/set-remove-merge.md`
- [x] `cypher/unwind-with-union.md`
- [x] `cypher/foreach-subquery.md`
- [x] `cypher/order-limit-skip.md`
- [x] `cypher/parameters.md`
- [x] `functions/string.md` / `math.md` / `list.md` / `predicates.md` / `aggregation.md` / `fulltext.md` / `temporal.md`
- [x] `indexes/property-index.md` / `fulltext-index.md` / `constraints.md`

## 注意
- `functions/fulltext.md` と `indexes/fulltext-index.md` の日本語形態素解析（IPADIC）の説明は、英語版でも「日本語テキスト向けの機能」として残す
- タスク 124（docs の未対応構文の解消）で日本語側が書き換わる可能性が高い。先に進んでいる方に合わせ、`source_commit` で追従する

## 完了条件
- 上記の `.en.md` があり、`mkdocs build --strict` と `docs_cypher_examples` テストが通る

## 結果（2026-11-12）
- 16 ページを翻訳。翻訳前に各ページの例を実機で確認し、日本語版の誤りを先に直した
  （where: 存在しない XOR → エンジンに実装、複数 MATCH の説明・例のコメント。unwind-with-union: UNION 後の ORDER BY は
  最後のクエリだけに効く → CALL {} で囲む例に。foreach-subquery・temporal: 例のコメント・出力。property-index: O(1)・SET 未対応の誤り、
  EXPLAIN の既知の不具合。fulltext-index / fulltext: SHOW INDEXES に全文検索は出ない、japanese feature の条件）
- 確認中に見つけて直した不具合: XOR、n[key] の動的プロパティ参照、MERGE のリレーションシップ変数（bug/165）、
  REPL でスキーマが消える（bug/166）、全文検索インデックスが SET / DELETE に追従しない（bug/168）。
  起票のみ: EXPLAIN がインデックスを反映しない（bug/167）
