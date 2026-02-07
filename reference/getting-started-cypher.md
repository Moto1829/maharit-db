# Getting Started Guide → Cypher

Source: https://neo4j.com/docs/getting-started/cypher/

## Cypherの位置づけ
- Neo4jの宣言的クエリ言語で、GQL準拠。
- openCypherとしてOSSでも提供。
- ASCIIアート風構文でパターンを記述し、CRUD操作が可能。

## Cypherの基礎構文
### ノード
- ノードは`(node)`の丸括弧。
- **ラベル**: `(:Person)` のように `:` で付与。
	- 同種ノードのグルーピング/最適化に寄与。
	- ラベル指定なしは全ノード走査になり得るため性能注意。
- **変数**: `(p:Person)` のように小文字推奨。
	- 以降の句で参照可能。

### リレーションシップ
- `-[]->` の角括弧と矢印で表現。
- **方向**
	- 左→右: `(p)-[:LIKES]->(t)`
	- 右→左: `(p)<-[:LIKES]-(t)`
	- 無向（探索方向不明時）: `(p)-[:LIKES]-(t)`
		- 無向は両方向探索になるため結果が重複する可能性。
- **タイプ**: `[:LIKES]` のように `:` 必須。
	- `[:LIKES]`はタイプ、`[LIKES]`は変数になる。
- **変数**: `(p)-[r:LIKES]->(t)` の `r`。

### プロパティ
- ノード/関係に `{key: value}` で付与。
- 値はシングル/ダブルクォートで文字列。
- 例: `(sally:Person {name:'Sally'})`。

### パターン
- パターンはCypherの中心概念。
- 例: `(sally:Person {name:"Sally"})-[:LIKES]->(t:Technology {type:"Graphs"})`
- `CREATE`で書き込み、`MATCH`で取得。
- **パターン変数**: `p = (sally)-[:LIKES]->(t)` のように全体に変数を割り当て可能。

## 追加リンク（詳細仕様）
- パターンの詳細: https://neo4j.com/docs/cypher-manual/current/patterns/reference/
- 句の一覧: https://neo4j.com/docs/cypher-manual/current/clauses/
- 値と型: https://neo4j.com/docs/cypher-manual/current/values-and-types/
- 関数: https://neo4j.com/docs/cypher-manual/current/functions/
