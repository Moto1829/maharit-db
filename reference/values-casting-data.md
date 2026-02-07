# Casting data values

Source: https://neo4j.com/docs/cypher-manual/current/values-and-types/casting-data/

## 代表的な変換関数
- `toBoolean`, `toFloat`, `toInteger`, `toString`
- `toBooleanList`, `toFloatList`, `toIntegerList`, `toStringList`
- `toBooleanOrNull`, `toFloatOrNull`, `toIntegerOrNull`, `toStringOrNull`

## 仕様
- 不正な型は `null` を返す関数と、例外を返す関数がある。
- Neo4j 2025.10 以降、`VECTOR` → `toFloatList`/`toIntegerList` をサポート。

## 用途
- 返却値の型変換、プロパティ更新時の型変更。
