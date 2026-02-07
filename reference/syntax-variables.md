# Variables

Source: https://neo4j.com/docs/cypher-manual/current/syntax/variables/

## 概要
- パターンやクエリの要素に名前を付けたものが変数。
- 例: `MATCH (n)-->(b) RETURN b` では `n` と `b` が変数。

## スコープ
- 変数は**同一クエリ部（query part）内のみ可視**。
- `WITH` で明示的に引き継がない限り次のクエリ部へは伝播しない。

## サブクエリ
- `CALL { ... }` に外側から渡した変数は**サブクエリ全体で可視**。
- サブクエリ内部の `WITH` に列挙せずとも参照可能。
- 参照: https://neo4j.com/docs/cypher-manual/current/subqueries/call-subquery/#variable-scope-clause
