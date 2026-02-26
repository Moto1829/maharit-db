# MERGE句

## 概要
MERGE句を実装する。MERGEはパターンがグラフに存在すればMATCHし、存在しなければCREATEする「upsert」操作を提供する。
ON CREATE SETとON MATCH SETによる条件付きプロパティ設定もサポートする。

## 実装内容

### AST拡張
- [x] `MergeClause` ステートメント型の追加
- [x] `ON CREATE SET` 句の追加
- [x] `ON MATCH SET` 句の追加

### レキサー拡張
- [x] `MERGE` キーワードトークンの追加

### パーサー拡張
- [x] `MERGE (n:Label {props})` パターンの認識
- [x] `MERGE (a)-[:REL]->(b)` リレーションシップパターンの認識
- [x] `ON CREATE SET` / `ON MATCH SET` のパース

### エグゼキュータ拡張
- [x] パターンの存在確認（MATCH試行）
- [x] 存在しない場合のCREATE実行
- [x] ON CREATE SET / ON MATCH SET の条件分岐実行
- [x] MATCH + MERGE の組み合わせ対応

## クエリ例
```cypher
-- ノードのupsert
MERGE (n:Person {name: "Alice"})
ON CREATE SET n.created = timestamp()
ON MATCH SET n.lastSeen = timestamp()
RETURN n

-- リレーションシップのupsert
MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"})
MERGE (a)-[:KNOWS]->(b)

-- MERGEで新規作成時にプロパティ設定
MERGE (n:Person {name: "Charlie"})
ON CREATE SET n.age = 25, n.city = "Osaka"
RETURN n
```

## 依存
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること
- `31-match-create.md` の設計を参考にする

## 対象クレート
`maharit-query`
