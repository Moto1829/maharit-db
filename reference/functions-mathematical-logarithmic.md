# Logarithmic functions

Source: https://neo4j.com/docs/cypher-manual/current/functions/mathematical-logarithmic/

## 関数
- `e()`, `exp(x)`, `log(x)`, `log10(x)`, `sqrt(x)`

## 重要ルール
- `null` 入力は `null`。
- `log(0)` / `log10(0)` は `-Infinity`。
- `log(x<0)` / `sqrt(x<0)` は `NaN`。
- `exp` が範囲超過すると `Infinity`。
