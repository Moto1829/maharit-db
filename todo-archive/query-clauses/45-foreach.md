# FOREACH句

## 概要
リストの各要素に対して副作用（CREATE, SET, REMOVE, DELETE等）を実行するFOREACH句を実装する。

## 実装内容

### FOREACH構文
- [x] FOREACH (variable IN list | update_clauses)
- [x] ネストしたFOREACH
- [x] リスト式・パラメータとの組み合わせ

### サポートする更新操作
- [x] FOREACH内でのCREATE
- [x] FOREACH内でのSET
- [x] FOREACH内でのREMOVE
- [x] FOREACH内でのDELETE
- [x] FOREACH内でのMERGE

## クエリ例
```cypher
-- リストからノード作成
FOREACH (name IN ['Alice', 'Bob', 'Charlie'] |
  CREATE (:Person {name: name})
)

-- マッチした結果に対する一括更新
MATCH p = (a:Person)-[:KNOWS*]->(b:Person)
FOREACH (n IN nodes(p) |
  SET n.visited = true
)

-- ネストしたFOREACH
FOREACH (city IN ['Tokyo', 'Osaka'] |
  FOREACH (name IN ['Alice', 'Bob'] |
    CREATE (:Person {name: name, city: city})
  )
)
```

## 依存
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること
- `41-list-operations.md` が完了していること

## 対象クレート
`maharit-query`
