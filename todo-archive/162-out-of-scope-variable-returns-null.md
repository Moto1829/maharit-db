# タスク 162: WITH で引き継がなかった変数を参照するとエラーにならず null になる

## 現象（2026-11-12、task124 で docs の例を実行して確認）
```text
MATCH (n:Person) WITH n.role AS r RETURN n.role
→ null（行数分）
```
`WITH` で投影しなかった `n` は以降のスコープに無いので、Cypher（Neo4j）では
`Variable 'n' not defined` のエラーになる。null が返るため、書き間違いに気付けない
（docs/functions/predicates.md の例も `WITH` で `dept` を落としていて、`dept.name` が null になっていた）。

## 作業
- [ ] プロパティ参照（`Expression::Property`）で未束縛の変数を参照したらエラーにする
  （OPTIONAL MATCH で null に束縛された変数とは区別する）
- [ ] WITH の後のスコープでも同様。既存テスト・docs の例で、たまたま null に頼っているものが無いか確認
