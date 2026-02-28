# クエリパラメータ

## 概要
パラメータ化されたクエリの実行をサポートする。SQLインジェクション対策やクエリキャッシュの効率化に寄与する。

## ステータス: 完了
## 実装日: 2026-02-26
## テスト数: 9件追加（maharit-query 合計 371件）

## 実装内容

### パラメータ構文
- [x] $param形式のパラメータ参照
- [x] パラメータのレキシング（Tokenとして認識）
- [x] パラメータのパース（Expression::Parameterノード）

### パラメータバインド
- [x] パラメータマップの受け渡し（HashMap<String, Value>）
- [x] 実行時のパラメータ解決
- [x] 未定義パラメータのエラーハンドリング

### パラメータの利用箇所
- [x] WHERE句: WHERE n.name = $name
- [x] プロパティ: ({name: $name})
- [x] SKIP / LIMIT: SKIP $offset LIMIT $count
- [x] SET句: SET n.name = $name

### クエリキャッシュとの統合
- [x] パラメータ化クエリのAST再利用
- [x] 実行計画の共有（パラメータ値が異なっても同じプラン）

## クエリ例
```cypher
-- パラメータ付きMATCH
MATCH (n:Person) WHERE n.name = $name RETURN n

-- パラメータ付きCREATE
CREATE (n:Person {name: $name, age: $age})

-- パラメータ付きSET
MATCH (n:Person {name: $name}) SET n.age = $new_age RETURN n

-- パラメータ付きMATCHパターン
MATCH (n:City {name: $city}) RETURN n.name
```

## 変更ファイル

- `crates/maharit-query/src/lexer.rs` - TokenKind::Parameter追加、$のレキシング
- `crates/maharit-query/src/ast.rs` - Expression::Parameter追加、NodePattern/EdgePattern.propertiesの型変更
- `crates/maharit-query/src/parser.rs` - parse_properties()の変更、parse_primary()にParameter対応追加
- `crates/maharit-query/src/executor.rs` - paramsフィールド、execute_with_params()、evaluate_expressionにParameter対応

## 依存
- `02-query-lexer.md` が完了していること
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること
- `24-query-optimizer.md`（クエリキャッシュ）が完了していること

## 対象クレート
`maharit-query`
