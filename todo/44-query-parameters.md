# クエリパラメータ

## 概要
パラメータ化されたクエリの実行をサポートする。SQLインジェクション対策やクエリキャッシュの効率化に寄与する。

## 実装内容

### パラメータ構文
- [ ] $param形式のパラメータ参照
- [ ] パラメータのレキシング（Tokenとして認識）
- [ ] パラメータのパース（Expression::Parameterノード）

### パラメータバインド
- [ ] パラメータマップの受け渡し（HashMap<String, Value>）
- [ ] 実行時のパラメータ解決
- [ ] 未定義パラメータのエラーハンドリング

### パラメータの利用箇所
- [ ] WHERE句: WHERE n.name = $name
- [ ] プロパティ: ({name: $name})
- [ ] SKIP / LIMIT: SKIP $offset LIMIT $count
- [ ] SET句: SET n.name = $name

### クエリキャッシュとの統合
- [ ] パラメータ化クエリのAST再利用
- [ ] 実行計画の共有（パラメータ値が異なっても同じプラン）

## クエリ例
```cypher
-- パラメータ付きMATCH
MATCH (n:Person) WHERE n.name = $name RETURN n

-- パラメータ付きCREATE
CREATE (n:Person {name: $name, age: $age})

-- パラメータ付きページネーション
MATCH (n:Person) RETURN n SKIP $offset LIMIT $limit

-- パラメータ付きSET
MATCH (n:Person {name: $name}) SET n.age = $new_age RETURN n
```

## 依存
- `02-query-lexer.md` が完了していること
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること
- `24-query-optimizer.md`（クエリキャッシュ）が完了していること

## 対象クレート
`maharit-query`
