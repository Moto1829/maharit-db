# Numeric functions

Source: https://neo4j.com/docs/cypher-manual/current/functions/mathematical-numeric/

## 関数
- `abs`, `ceil`, `floor`, `isNaN`, `rand`, `round`, `sign`

## 重要ルール
- `round(value[, precision, mode])`（UP/DOWN/CEILING/FLOOR/HALF_UP/HALF_DOWN/HALF_EVEN）。
- `isNaN` で NaN 判定。
- `null` 入力は `null`。
