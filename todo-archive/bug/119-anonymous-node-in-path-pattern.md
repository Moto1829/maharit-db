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

- [x] `MATCH (:A)-[r]->(:B)`、`MATCH (a)-[:R]->(:B)`、`MATCH (:A)-[:R]->(b)` が動作
- [x] 匿名ノードが結果カラムに現れない

## 完了内容 (2026-10-01)

### 原因と、より深刻だった関連バグ
`match_path_pattern` が各セグメントの「前のノード」として **常にパターン先頭のノード** を渡していた。
- 先頭が匿名 `(:A)` → `path pattern requires variable` エラー（本タスク）
- **名前付きでも 2 ホップ以上は誤結果**: `(a)-[:KNOWS]->(b)-[:KNOWS]->(c)` の 2 ホップ目も `a` から辿っていた
  （期待 `[Alice,Bob,Carol]` に対し `[Alice,Bob,Bob]`, `[Bob,Carol,Carol]` を黙って返していた）

### 修正
- 各ホップは直前のセグメントのノードから辿る
- 匿名ノードにはクエリから書けない内部変数（`\0anon…`）を割り当て、パス照合後にバインディングから除去（`RETURN *` に出ない）
- あわせて **パス変数** `MATCH p = (…)-[…]->(…)` を実装（匿名エッジにも内部変数を割り当て、可変長ホップも連結して `Path` を構築）

### Cypher 適合テスト（e2e/121-4）で見つけて併せて実装
`crates/maharit-query/src/conformance_tests.rs`（固定グラフ＋期待結果の表、50 ケース）で以下が失敗していたため実装:
- `IS NULL` / `IS NOT NULL`（パースすらできなかった。`IS NORMALIZED` のみ対応）
- WHERE など式中の任意のスカラー関数（`WHERE toLower(n.name) = 'x'`, `WHERE size(n.name) > 3`）
- RETURN 項目の演算式（`RETURN n.age + 1`, `toUpper(n.name) = 'BOB' AS b`）、RETURN の CASE / パラメータ / 括弧式
- 文字列の `+` 連結（文字列同士、文字列と数値）
- `EXISTS { (n)-->() }` / `COUNT { … }` の MATCH 省略形
