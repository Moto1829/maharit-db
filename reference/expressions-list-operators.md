# List operators

Source: https://neo4j.com/docs/cypher-manual/current/expressions/predicates/list-operators/

## 演算子
- メンバーシップ: `IN`

## 挙動
- `IN` は要素が**少なくとも1つ存在**すれば `true`。
- 重複要素の有無は結果に影響しない。
- `null` が関与すると結果は `null`（`value IN null` / `null IN [1,2,null]`）。
- ネストされたリストでは**完全一致**のみ `true`。

## 部分集合
- `all(x IN sub WHERE x IN list)` で部分集合判定。
- 追加の述語は `all/any/none/single/isEmpty` 関数を使用。
