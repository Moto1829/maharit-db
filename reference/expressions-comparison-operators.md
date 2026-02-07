# Comparison operators

Source: https://neo4j.com/docs/cypher-manual/current/expressions/predicates/comparison-operators/

## 演算子
- `=`, `<>`, `<`, `>`, `<=`, `>=`, `IS NULL`, `IS NOT NULL`

## 連鎖比較
- `a < b = c <= d` は `a < b AND b = c AND c <= d` と同等。
- `a op1 b op2 c` は `a` と `c` を直接比較しない。

## 参考
- 型の比較順序と等価性: https://neo4j.com/docs/cypher-manual/current/values-and-types/ordering-equality-comparison/
