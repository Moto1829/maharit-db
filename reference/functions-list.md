# List functions

Source: https://neo4j.com/docs/cypher-manual/current/functions/list/

## 代表関数
- `coll.distinct`, `coll.flatten`, `coll.indexOf`, `coll.insert`, `coll.remove`, `coll.sort`
- `coll.max`, `coll.min`
- `keys`, `labels`, `nodes`, `relationships`
- `range`, `reduce`, `reverse`, `tail`
- `toBooleanList`, `toFloatList`, `toIntegerList`, `toStringList`

## 重要ルール
- `coll.*` は `null` 入力で `null`。
- `coll.max/min/sort` は Cypher の比較順序に従う。
- `toFloatList`/`toIntegerList` は Neo4j 2025.10 以降 `VECTOR` も受け付け。
