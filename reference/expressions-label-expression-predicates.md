# Label expression predicates

Source: https://neo4j.com/docs/cypher-manual/current/expressions/predicates/label-expression-predicates/

## 構文
- `<expr> : <label-expression>`
- `<expr>` はノード/関係式、`<label-expression>` はラベル式。

## ノードのラベル判定
- `p:Manager` は `p` がラベル `Manager` を持つか判定。
- `p:Manager|Director|CEO` はOR判定。
- `p:!CEO` は否定。
- `p` が `null` の場合は `null`。

## 動的ラベル式（Cypher 25）
- `p:$any(list)` のように動的ラベル式を利用可能（Neo4j 2025.07）。

## 関係タイプ判定
- `r:WORKS_FOR` で関係タイプ判定。
- `r` が `null` の場合は `null`。

## 補助
- `coalesce()` で `null` を `false` などに置換可能。
