# WHERE 句内パターン述語

**Status**: Not Started

## 概要
`WHERE` 句の中でグラフパターンを条件式として使用できるようにする。
Neo4j Cypher では `WHERE (n)-->()` や `WHERE NOT (n)-[:KNOWS]->(m)` のような
パターン述語（Pattern Predicates）が広く使われる。

## 現状の問題

現在 `WHERE` 句はスカラー式（比較・論理演算等）のみ対応。
グラフパターンを真偽値として評価する機能が未実装。

## 実装内容

### AST 拡張

- [ ] `Expression::PatternPredicate(PatternPredicateExpr)` を追加
- [ ] `PatternPredicateExpr` 構造体: パターン + 変数バインディング（変数なしの匿名パターンも可）

### パーサー拡張

- [ ] `WHERE` 内でのパターン記法 `(n)-->()` を式としてパースできるようにする
- [ ] `NOT` との組み合わせ: `WHERE NOT (n)-[:KNOWS]->(m)`
- [ ] 既存の `exists()` サブクエリ構文との整合性を保つ

### エグゼキュータ拡張

- [ ] パターン述語をバインディングに対して評価する（マッチすれば `true`）
- [ ] 既存 `match_pattern()` を再利用してパターンを評価

## クエリ例

```cypher
-- 友人を持つ人のみ返す
MATCH (n:Person)
WHERE (n)-[:KNOWS]->()
RETURN n.name

-- 友人を持たない人を返す
MATCH (n:Person)
WHERE NOT (n)-[:KNOWS]->()
RETURN n.name

-- 特定のノードとの関係があるか
MATCH (a:Person), (b:Person)
WHERE (a)-[:KNOWS]->(b)
RETURN a.name, b.name

-- 型付きパターン述語
MATCH (n:Person)
WHERE (n)-[:WORKS_AT]->(:Company {name: "ACME"})
RETURN n.name
```

## 依存

- `03-query-parser.md` が完了していること
- `43-subqueries.md`（EXISTS サブクエリ）が完了していること

## 対象クレート

`maharit-query`
