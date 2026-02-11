# サブクエリ

## 概要
Cypherのサブクエリ機能を実装する。クエリの中にネストしたクエリを記述できるようにする。

## 実装内容

### CALLサブクエリ
- [ ] CALL { subquery }: インラインサブクエリ
- [ ] 外部変数のインポート
- [ ] UNION内でのサブクエリ

### EXISTSサブクエリ
- [ ] EXISTS { MATCH pattern }: パターンの存在チェック
- [ ] WHERE句内での使用

### COUNTサブクエリ
- [ ] COUNT { MATCH pattern }: パターンのマッチ数
- [ ] WHERE句・RETURN句での使用

### COLLECTサブクエリ
- [ ] COLLECT { MATCH pattern RETURN expr }: サブクエリ結果のリスト化

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

## 依存
- `23-query-advanced.md` が完了していること（WITH句）
- `04-query-executor.md` が完了していること

## 対象クレート
`maharit-query`
