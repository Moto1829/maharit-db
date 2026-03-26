# Task 43: サブクエリ

## 概要
Cypherのサブクエリ機能を実装する。クエリの中にネストしたクエリを記述できるようにする。

## 実装内容

### CALLサブクエリ
- [x] CALL { subquery }: インラインサブクエリ
- [x] 外部変数のインポート（WITH句）
- [x] 内部RETURN項目のエイリアス（AS）対応

### EXISTSサブクエリ
- [x] EXISTS { MATCH pattern }: パターンの存在チェック
- [x] WHERE句内での使用
- [x] 内部WHERE句（EXISTS { MATCH ... WHERE ... }）

### COUNTサブクエリ
- [x] COUNT { MATCH pattern }: パターンのマッチ数
- [x] WHERE句・RETURN句での使用

### COLLECTサブクエリ
- [x] COLLECT { MATCH pattern RETURN expr }: サブクエリ結果のリスト化

## クエリ例
```cypher
-- CALLサブクエリ
MATCH (p:Person)
CALL {
  WITH p
  MATCH (p)-[:KNOWS]->(f:Person)
  RETURN COUNT(f) AS friend_count
}
RETURN p.name, friend_count

-- EXISTSサブクエリ
MATCH (p:Person)
WHERE EXISTS {
  MATCH (p)-[:KNOWS]->(:Person {city: 'Tokyo'})
}
RETURN p.name

-- COUNTサブクエリ
MATCH (p:Person)
WHERE COUNT {
  MATCH (p)-[:KNOWS]->()
} > 5
RETURN p.name AS popular_person

-- COLLECTサブクエリ
MATCH (p:Person)
RETURN p.name, COLLECT {
  MATCH (p)-[:KNOWS]->(f:Person)
  RETURN f.name
} AS friend_names
```

## 実装詳細

### AST変更（ast.rs）
- `SubqueryPattern`: EXISTS/COUNTサブクエリのパターン部
- `CollectSubqueryBody`: COLLECTサブクエリの本体
- `CallSubquery`: CALLサブクエリ（with_import、match_clause、where_clause、return_items）
- `CallReturnItem`: CALLサブクエリのRETURN項目（エイリアスあり）
- `MatchStatement.call_clause`: オプショナルなCALL句フィールド
- `Expression::ExistsSubquery`、`CountSubquery`、`CollectSubquery`: 新しい式バリアント

### レクサー変更（lexer.rs）
- `TokenKind::Call`: `CALL`キーワードを追加

### パーサー変更（parser.rs）
- `parse_call_subquery()`: CALL { ... }のパース
- `parse_call_return_item()`: AS付きRETURN項目のパース
- `parse_subquery_pattern()`: EXISTS/COUNT用パターンパース
- `parse_collect_subquery_body()`: COLLECT用ボディパース
- `parse_primary()`、`parse_return_item()`: EXISTS/COUNT/COLLECTキーワード+`{`検出

### エグゼキュータ変更（executor.rs）
- `execute_match()`: call_clause実行対応
- `execute_call_subquery()`: CALL実行ロジック（外部バインディングとのマージ）
- `evaluate_expression()`: ExistsSubquery、CountSubquery、CollectSubquery評価

### プランナー変更（planner.rs）
- `plan_match()`: call_clause用CallSubqueryプランノード追加

## 依存
- `23-query-advanced.md` が完了していること（WITH句）
- `04-query-executor.md` が完了していること

## 対象クレート
`maharit-query`

## テスト
- 22件の新規テスト追加（パーサー10件、エグゼキュータ12件）
- 合計337テスト（maharit-query）
