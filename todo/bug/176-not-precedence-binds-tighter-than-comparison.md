# バグ 176: `NOT` が比較演算子より強く結合し、`WHERE NOT a = b` が常に 0 件になる

## 現状（2026-11-15、main 60302578 で確認）
```text
CREATE (:P {name:'A'}), (:P {name:'B'})
MATCH (p:P) WHERE NOT p.name = 'A' RETURN p.name            // => (no results)  期待: "B"
MATCH (p:P) WHERE NOT (p.name = 'A') RETURN p.name          // => "B"（括弧を付けると正しい）
MATCH (p:P) WHERE NOT p.name IS NULL RETURN p.name          // => (no results)  期待: "A","B"
MATCH (p:P) WHERE NOT p.name STARTS WITH 'X' RETURN p.name  // => (no results)  期待: "A","B"
MATCH (p:P) RETURN NOT p.name = 'A' AS x                    // => type error: NOT requires boolean
```

## 原因
`parser.rs` の `parse_unary` が `NOT` を単項マイナスと同じ最上位の優先度で読むため、
`NOT p.name = 'A'` は `(NOT p.name) = 'A'` になる。`NOT` に文字列を渡すと型エラーになり、
WHERE がエラーを `unwrap_or(false)` で偽として扱うので、エラーにならずに全行が落ちる。

Cypher の優先順位は `OR` < `XOR` < `AND` < `NOT` < 比較（`=` `<>` `<` `IS NULL` `STARTS WITH` `IN` …）< 加減 < 乗除 < 単項マイナス。

## 作業
- [ ] `parse_and_expression` と `parse_comparison` の間に `parse_not`（`NOT` を繰り返し読んで `parse_comparison` へ）を入れ、`parse_unary` から `NOT` を外す
- [ ] 上の例を conformance_tests に追加（`NOT NOT x`、`NOT a AND b` = `(NOT a) AND b` も）
- [ ] パターン述語 `WHERE NOT (a)-->(b)` や `NOT EXISTS {…}` が引き続き動くことを確認
- [ ] 利用者から見て結果が変わる修正なので `fix(query)!:` で出す
