# タスク 205: 比較の意味を標準に合わせる（真偽値・リストの大小、NaN、未定義の変数）

## 背景（2026-11-21、openCypher TCK）

```text
RETURN false < true AS r                 // => null   期待: true（真偽値は false < true）
RETURN [1, 2] < [1, 3] AS r              // => null   期待: true（リストは要素ごとに辞書順）
RETURN 0.0 / 0.0 = 0.0 / 0.0 AS r        // NaN の比較: =, <, >, <=, >= は全て false、<> は true
RETURN 1 < 'a'                           // null（型が違う。今と同じ）
MATCH (n) WHERE n.x = undefinedVar ...   // 未定義の変数は実行前にエラー（今は 0 件）
```

TCK: `comparison/Comparison1 [17]`、`Comparison2 [3] [4] [5]`、`precedence/Precedence1 [6]`。
`Comparison2 [3]` は「型が違う比較は null、ただし整数と浮動小数は比べる」の確認で、WHERE で行が残らない。原因を調べる。

## 作業
- [ ] `compare_values` に真偽値とリスト（要素ごと、null を含めば null）を足す
- [ ] NaN の扱い（`partial_cmp` が None のとき）を標準に合わせる
- [ ] 未定義の変数を実行前（解析後）に検出してエラーにする。WHERE がエラーを偽にして隠さないこと（bug/177 で残した判断も合わせて見直す）
- [ ] ORDER BY の並び（`compare_values_for_sort`）と矛盾しないか確かめる
- [ ] 結果が変わるので `fix(query)!:`
