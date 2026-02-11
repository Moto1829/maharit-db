# スカラー関数

## 概要
Cypherの組み込みスカラー関数を実装する。ノード・エッジのメタデータ取得や型変換を行う。

## 実装内容

### NULL処理
- [ ] coalesce(expr, expr, ...): 最初の非NULL値を返す
- [ ] nullIf(expr1, expr2): 2つが等しければNULLを返す

### ノード・エッジ情報
- [ ] id(node/edge): IDの取得
- [ ] elementId(node/edge): 文字列IDの取得
- [ ] type(edge): エッジタイプの取得
- [ ] startNode(edge): エッジの始点ノード
- [ ] endNode(edge): エッジの終点ノード
- [ ] labels(node): ノードのラベルリスト

### プロパティ操作
- [ ] properties(node/edge): プロパティMapの取得
- [ ] keys(node/edge/map): キーのリスト取得

### 型変換
- [ ] toBoolean(value): ブール値へ変換
- [ ] toFloat(value): 浮動小数点へ変換
- [ ] toInteger(value): 整数へ変換
- [ ] toString(value): 文字列へ変換

### ユーティリティ
- [ ] timestamp(): 現在のUnixタイムスタンプ（ミリ秒）
- [ ] randomUUID(): ランダムUUIDの生成

## クエリ例
```cypher
-- coalesce
MATCH (n:Person) RETURN coalesce(n.nickname, n.name) AS display_name

-- type
MATCH (a)-[r]->(b) RETURN type(r) AS relationship_type

-- properties
MATCH (n:Person {name: 'Alice'}) RETURN properties(n) AS props

-- keys
MATCH (n:Person) RETURN keys(n) AS property_names

-- 型変換
MATCH (n:Person) RETURN toInteger(n.age_string) AS age

-- id
MATCH (n:Person) RETURN id(n) AS node_id
```

## 依存
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること

## 対象クレート
`maharit-query`
