# タスク 119: 変数なしノードを含むパスパターンが `path pattern requires variable` エラーになる

## 概要

標準 Cypher で一般的な書き方がエラーになる。

```cypher
MATCH (:T)-[r:K]->(:T) RETURN r.weight
-- Execution error: type error: path pattern requires variable
MATCH (a:T)-[r:K]->(b:T) RETURN r.weight   -- こちらは成功
```

task116 の E2E 作成時に発見（replication_test.py は変数付きで回避済み）。

## 修正方針（案）

- executor のパターンマッチでノード変数が `None` の場合に内部用の匿名変数名（例: `  anon_0`）を割り当て、
  RETURN * 等の結果カラムには出さない
- エッジの変数なし（`-[:K]->`）は既に動くので、同様の扱いをノードにも適用

## 受け入れ条件

- [ ] `MATCH (:A)-[r]->(:B)`、`MATCH (a)-[:R]->(:B)`、`MATCH (:A)-[:R]->(b)` が動作
- [ ] 匿名ノードが結果カラムに現れない
