# Working with null

Source: https://neo4j.com/docs/cypher-manual/current/values-and-types/working-with-null/

## 概要
- `null` は欠損/不明の値。
- すべての型は `null` を含む。

## 主要な挙動
- `null = null` は `null`。
- `WHERE` では `true` 以外はフィルタ対象。
- `IS NULL` / `IS NOT NULL` を使用。

## 論理演算
- `AND`/`OR`/`XOR`/`NOT` は三値論理。

## `IN` と `null`
- 要素が確定して一致する場合のみ `true`。
- 不確定なら `null`。

## `[]` と `null`
- `list[null]` / `map[null]` は `null`。
- 範囲の `null` は `coalesce()` で回避可能。

## 例
- 存在しないプロパティ参照は `null`。
- `1 + null` や `sin(null)` は `null`。
