# Task 35: UNWIND句

## 概要
リスト（配列）を行に展開するUNWIND句を実装する。
バッチデータの投入やリスト操作と組み合わせたCREATE/MERGEに利用される。

## 実装内容

### AST拡張
- [x] `UnwindClause` の追加（式 + AS 変数名）

### レキサー拡張
- [x] `UNWIND` キーワードトークンの追加

### パーサー拡張
- [x] `UNWIND expr AS var` のパース
- [x] UNWIND後のCREATE / MERGE / RETURN との接続

### エグゼキュータ拡張
- [x] リスト式の評価
- [x] リスト要素ごとに行を展開してバインディング生成
- [x] 後続句（RETURN / CREATE / MERGE）への引き渡し

## クエリ例
```cypher
-- リストの展開
UNWIND [1, 2, 3] AS x
RETURN x

-- バッチノード作成
UNWIND [{name: "Alice"}, {name: "Bob"}] AS props
CREATE (n:Person) SET n = props
RETURN n

-- MATCH結果のリストプロパティを展開
MATCH (n:Person {name: "Alice"})
UNWIND n.hobbies AS hobby
RETURN hobby

-- COLLECT結果の再展開
MATCH (n:Person)
WITH COLLECT(n.name) AS names
UNWIND names AS name
RETURN name
```

## 依存
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること

## 対象クレート
`maharit-query`
