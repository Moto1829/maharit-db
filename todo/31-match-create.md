# MATCH + CREATE 複合クエリ

## 概要
MATCHで取得した結果をCREATEで利用する複合クエリを実装する。
既存ノード間のリレーションシップ作成や、マッチ結果に基づく新規ノード作成を1つのクエリで実行可能にする。

## 実装内容

### AST拡張
- [ ] `MatchCreateStatement` 複合ステートメント型の追加
- [ ] CREATE句でMATCHの変数バインディングを参照可能にする

### パーサー拡張
- [ ] `MATCH ... CREATE ...` パターンの認識
- [ ] `MATCH ... WHERE ... CREATE ...` パターンの認識
- [ ] CREATE句内でのバインド済み変数と新規ノードの区別

### エグゼキュータ拡張
- [ ] MATCHフェーズのバインディングをCREATEコンテキストに引き渡す
- [ ] バインド済み変数は既存ノードとして扱い、新規作成しない
- [ ] 複数マッチ行に対するCREATEの繰り返し実行

## クエリ例
```cypher
-- 既存ノード間にリレーションシップを作成
MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"})
CREATE (a)-[:KNOWS]->(b)

-- マッチ結果に基づいて新規ノードとリレーションシップを作成
MATCH (a:Person {name: "Alice"})
CREATE (a)-[:OWNS]->(c:Car {model: "Tesla"})

-- WHERE条件付き
MATCH (a:Person) WHERE a.age > 20
CREATE (a)-[:MEMBER_OF]->(g:Group {name: "Adults"})
```

## 依存
- `03-query-parser.md` が完了していること
- `04-query-executor.md` が完了していること

## 対象クレート
`maharit-query`
