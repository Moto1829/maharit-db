# タスク 118: `MATCH … SET … REMOVE …` の REMOVE 部分がエラーなく無視される

## 概要

`MatchSetStatement` は `set_clause` しか持たず（`crates/maharit-query/src/ast.rs`）、
SET の後ろに続く REMOVE 句はパースエラーにもならず黙って捨てられる。

```cypher
CREATE (:P {name: 'a', age: 1})
MATCH (n:P) SET n.age = 2 REMOVE n.name   -- 成功扱いだが n.name は残る
```

task116 のテスト作成時に発見。ユーザーから見ると「成功したのに反映されない」ため危険。

## 修正方針（案）

- パーサー: 文末に未消費トークンが残る場合はエラーにする（他の文でも同種の取りこぼしがないか確認）
- SET / REMOVE の任意の並び（`SET … REMOVE … SET …`）を `Vec<UpdateClause>` として保持・順に適用

## 受け入れ条件

- [ ] `MATCH (n) SET … REMOVE …` で両方が適用される
- [ ] 解釈できない後続トークンがある場合はパースエラーになる
