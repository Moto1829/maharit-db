# MATCH + REMOVE 複合クエリ

## 概要
MATCHで取得したノード・エッジからプロパティやラベルを削除するREMOVE句を実装する。
SET句の逆操作にあたる。

## 実装内容

### AST拡張
- [x] `RemoveClause` の追加
- [x] `RemoveItem`（プロパティ削除 / ラベル削除）の定義

### レキサー拡張
- [x] `REMOVE` キーワードトークンの追加

### パーサー拡張
- [x] `MATCH ... REMOVE ... RETURN ...` パターンの認識
- [x] プロパティ削除: `REMOVE n.prop` のパース
- [x] ラベル削除: `REMOVE n:Label` のパース

### エグゼキュータ拡張
- [x] MATCHバインディングに基づくプロパティ削除
- [x] ラベル削除の実行
- [x] 削除後の結果をRETURN

## クエリ例
```cypher
-- プロパティの削除
MATCH (n:Person {name: "Alice"})
REMOVE n.age
RETURN n

-- ラベルの削除
MATCH (n:Person:Adult)
REMOVE n:Adult
RETURN n

-- 複数プロパティの削除
MATCH (n:Person {name: "Alice"})
REMOVE n.age, n.city
RETURN n

-- リレーションシップのプロパティ削除
MATCH (a)-[r:KNOWS]->(b)
REMOVE r.since
RETURN a.name, b.name
```

## 依存
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること

## 対象クレート
`maharit-query`
