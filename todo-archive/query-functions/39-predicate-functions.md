# 述語関数

## 概要
リストやコレクションに対する述語関数を実装する。

## 実装内容

### リスト述語関数
- [x] all(variable IN list WHERE predicate): 全要素が条件を満たすか
- [x] any(variable IN list WHERE predicate): いずれかの要素が条件を満たすか
- [x] none(variable IN list WHERE predicate): 全要素が条件を満たさないか
- [x] single(variable IN list WHERE predicate): ちょうど1つの要素が条件を満たすか

### 存在チェック
- [x] exists(property): プロパティが存在するか
- [x] isEmpty(list/string/map): 空かどうか

## クエリ例
```cypher
-- all: 全友人が30歳以上か
MATCH (p:Person)-[:KNOWS]->(f:Person)
WITH p, COLLECT(f.age) AS ages
WHERE all(age IN ages WHERE age >= 30)
RETURN p.name

-- any: いずれかの友人が東京在住か
MATCH (p:Person)-[:KNOWS]->(f:Person)
WITH p, COLLECT(f.city) AS cities
WHERE any(city IN cities WHERE city = 'Tokyo')
RETURN p.name

-- none: 30歳未満の友人がいないか
MATCH (p:Person)-[:KNOWS]->(f:Person)
WITH p, COLLECT(f.age) AS ages
WHERE none(age IN ages WHERE age < 30)
RETURN p.name

-- exists
MATCH (n:Person) WHERE exists(n.email) RETURN n.name

-- isEmpty
MATCH (n:Person) WHERE NOT isEmpty(n.hobbies) RETURN n.name
```

## 依存
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること
- `41-list-operations.md` のリスト基盤が望ましい

## 対象クレート
`maharit-query`
