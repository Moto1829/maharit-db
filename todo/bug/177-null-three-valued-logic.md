# バグ 177: null の三値論理が無く、WHERE で行が消えたり残ったりする

## 現状（2026-11-15、main 60302578 で確認）
```text
CREATE (:P {name:'A', age:30}), (:P {name:'B', age:20}), (:P {name:'C'})
MATCH (p:P) WHERE p.name = 'C' OR p.age >= 20 RETURN p.name   // => "A","B"     期待: "A","B","C"
MATCH (p:P) WHERE p.age <> 30 RETURN p.name                   // => "B","C"     期待: "B"
MATCH (p:P) WHERE p.age IN [30, null] RETURN p.name           // => "A","C"     期待: "A"
MATCH (p:P) RETURN p.name, p.age > 25 AS old                  // => type error: cannot compare values（クエリ全体が失敗）
MATCH (p:P) RETURN p.name, p.age + 1 AS n                     // => type error: arithmetic requires numbers
RETURN null = null AS x       // => true   期待: null
RETURN 1 + null AS x          // => type error  期待: null
RETURN true OR null AS x      // => type error  期待: true
RETURN false AND null AS x    // => type error  期待: false
RETURN NOT null AS x          // => type error  期待: null
```
1 件目が特に危ない。左辺が true でも右辺が null 比較でエラーになり、WHERE がエラーを偽として扱うため、
条件を満たす行が黙って結果から消える。

## 原因（executor.rs）
- `values_equal` が `(Null, Null) => true`、`Eq` / `Neq` はその bool をそのまま返す（null を返さない）
- `compare_values` は null が来るとエラー、`arithmetic_op` も null でエラー
- `And` / `Or` / `Not` は `Value::Bool` 以外でエラー（null を通さない）
- `In` は右のリストに null があっても true / false しか返さない
- WHERE の各所で `evaluate(...).map(|v| matches!(v, Bool(true))).unwrap_or(false)` としているため、上のエラーが隠れる

## 作業
- [ ] `=` / `<>` / `<` `>` `<=` `>=`：どちらかが null なら null
- [ ] `+ - * /`（数値・文字列・リスト・テンポラル）：どちらかが null なら null
- [ ] `AND` / `OR` / `NOT`（`XOR` は未実装）：Kleene の三値論理（`false AND null = false`、`true OR null = true`、それ以外の null 混じりは null）
- [ ] `IN`：一致が無くリストに null がある、または左辺が null なら null
- [ ] `STARTS WITH` / `ENDS WITH` / `CONTAINS` / `=~`：null なら null
- [ ] `values_equal` を等価判定（DISTINCT・グルーピング・`IN` の一致探索）用と、`=` の評価用に分ける（DISTINCT / GROUP BY では null 同士を同じ値として扱うのが Cypher の意味）
- [ ] WHERE の `unwrap_or(false)` は本物の型エラーまで隠すので、null は偽、型エラーはエラーとして返すかを決める
- [ ] conformance_tests に上の例を追加。プランナのフィルタプッシュダウン・プロパティ索引の等価/範囲検索で null の扱いが変わらないか確認
- [ ] 同じクエリで結果が変わるので `fix(query)!:` で出す
