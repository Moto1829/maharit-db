# MATCH + SET 複合クエリ

## 概要
MATCHで取得したノード・エッジのプロパティをSET句で更新する複合クエリを実装する。
現在SETはDELETE文の前処理としてのみ動作するが、単独のMATCH + SET + RETURNをサポートする。

## 実装内容

### AST拡張
- [ ] SET句を持つ独立したステートメント型の追加（またはMatchStatementの拡張）
- [ ] SET句での`+=`（プロパティのマージ）対応

### パーサー拡張
- [ ] `MATCH ... SET ... RETURN ...` パターンの認識
- [ ] `MATCH ... WHERE ... SET ... RETURN ...` パターンの認識
- [ ] DELETE文以外でのSET句パース対応

### エグゼキュータ拡張
- [ ] MATCHバインディングに基づくプロパティ更新
- [ ] 更新結果のRETURN対応
- [ ] ラベルの追加: `SET n:NewLabel`

## クエリ例
```cypher
-- プロパティの更新
MATCH (n:Person {name: "Alice"})
SET n.age = 31
RETURN n

-- 複数プロパティの更新
MATCH (n:Person {name: "Alice"})
SET n.age = 31, n.city = "Tokyo"
RETURN n

-- プロパティのマージ
MATCH (n:Person {name: "Alice"})
SET n += {age: 31, city: "Tokyo"}
RETURN n

-- ラベルの追加
MATCH (n:Person) WHERE n.age >= 20
SET n:Adult
RETURN n

-- リレーションシップのプロパティ更新
MATCH (a:Person)-[r:KNOWS]->(b:Person)
SET r.since = 2024
RETURN a.name, b.name, r.since
```

## 依存
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること

## 対象クレート
`maharit-query`
