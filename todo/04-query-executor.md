# クエリ実行エンジン

## 概要
パーサーが生成したASTを解釈し、グラフに対する操作を実行するエンジンを実装する。

## 実装内容

### 実行コンテキスト
- [x] 変数バインディングの管理
- [x] スコープ管理

### CREATE実行
- [x] ノード作成: `CREATE (n:Person {name: "Alice"})`
- [x] エッジ作成: `CREATE (a)-[:KNOWS]->(b)`
- [x] 複数要素の一括作成

### MATCH実行
- [x] 単一ノードのマッチ: `MATCH (n:Person)`
- [x] パターンマッチング: `MATCH (a)-[:KNOWS]->(b)`
- [ ] 複数ホップのパス: `MATCH (a)-[:KNOWS*2..3]->(b)`

### WHERE実行
- [x] プロパティ比較: `WHERE n.age > 20`
- [x] 論理演算: `WHERE n.age > 20 AND n.name = "Alice"`

### RETURN実行
- [x] 変数の返却: `RETURN n`
- [x] プロパティの返却: `RETURN n.name, n.age`
- [ ] 集計関数: `COUNT()`, `SUM()`, `AVG()`

### DELETE実行
- [x] ノード削除: `DELETE n`
- [x] エッジ削除: `DELETE r`
- [x] DETACH DELETE（関連エッジも削除）

## 依存
- `03-query-parser.md` が完了していること

## 対象クレート
`maharit-query`
