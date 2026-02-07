# Fixed-length Patterns

Source: https://neo4j.com/docs/cypher-manual/current/patterns/fixed-length-patterns/

## Node patterns
- `MATCH ()` は全ノードに一致。
- 変数を付けると参照可能（例: `MATCH (n)`）。
- ラベル式でフィルタ（例: `MATCH (n:Stop)`）。
- プロパティ・マップで等価条件（例: `(n {mode:'Rail'})`）。
- `WHERE` 句で一般的な条件を付与可能。

## Relationship patterns
- `--` は方向/型/プロパティを指定しない関係に一致。
- `-[r]-` で変数を束縛。
- `-[r]->` / `<-[r]-` で方向指定。
- `-[:TYPE]->` でタイプ指定。
- `[{...}]` でプロパティ一致。
- `WHERE` による一般条件。

## Path patterns
- 最低1ノードを含み、ノードと関係が交互。
- 先頭/末尾は必ずノード。
- 例: `()`、`(s)--(e)`、`(:Station)--()<--(m WHERE m.departs > time('12:00'))-->()-[:NEXT]->(n)`

## パターンマッチの例
- `MATCH (s:Stop)-[:CALLS_AT]->(:Station {name:'Denmark Hill'}) RETURN s.departs` など。

## 参照
- Node patterns: https://neo4j.com/docs/cypher-manual/current/patterns/reference/#node-patterns
- Relationship patterns: https://neo4j.com/docs/cypher-manual/current/patterns/reference/#relationship-patterns
