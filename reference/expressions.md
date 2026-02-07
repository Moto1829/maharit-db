# Expressions

Source: https://neo4j.com/docs/cypher-manual/current/expressions/

## 概要
- Cypher式は「値に評価されるクエリの一部」。

## 主なカテゴリ
- Predicates
	- Boolean operators: `AND`, `OR`, `XOR`, `NOT`
	- Comparison operators: `=`, `<>`, `<`, `>`, `<=`, `>=`, `IS NULL`, `IS NOT NULL`
	- List operators: `IN`
	- String operators: `STARTS WITH`, `ENDS WITH`, `CONTAINS`, `IS NORMALIZED`, `IS NOT NORMALIZED`, `=~`
	- Label expression predicates
	- Path pattern expressions
	- Type predicate expressions
- Node and relationship operators: `.` / `[]` によるプロパティ参照
- Mathematical operators: `+`, `-`, `*`, `/`, `%`, `^`
- String concatenation operators: `+`, `||`
- Temporal operators: `+`, `-`, `*`, `/`
- List expressions: 結合、要素アクセス、スライス、内包表記、パターン内包
- Map expressions: `.` / `[]`、map projection
- Conditional expressions: `CASE`

## 他章に定義される式
- Label expressions: https://neo4j.com/docs/cypher-manual/current/patterns/reference/#label-expressions
- Function calls: https://neo4j.com/docs/cypher-manual/current/functions/
- Subquery expressions: `COLLECT`, `COUNT`, `EXISTS`
- Value literals: https://neo4j.com/docs/cypher-manual/current/values-and-types/
- Graph references: https://neo4j.com/docs/cypher-manual/current/values-and-types/graph-references/

## セキュリティ注意
- ユーザー入力を無加工で埋め込むとCypherインジェクションのリスク。
- [Parameters](https://neo4j.com/docs/cypher-manual/current/syntax/parameters/)の利用が推奨。
