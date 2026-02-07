# Boolean operators

Source: https://neo4j.com/docs/cypher-manual/current/expressions/predicates/boolean-operators/

## 演算子
- `AND`, `OR`, `XOR`, `NOT`

## 三値論理
- `null` を含む論理演算をサポート（`true/false/null`）。
- 例: `true AND null` → `null`、`false AND null` → `false`。

## 備考
- 複合条件は括弧で明示すると誤解がない。
