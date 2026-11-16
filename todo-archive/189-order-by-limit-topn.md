# 189: ORDER BY … LIMIT で全件を並べ替えない

## 問題

`read/sort/order_by_limit`（`ORDER BY p.age DESC LIMIT 10`）が 8.61 ms で、全件の並べ替え（8.90 ms）とほぼ同じ。`scan/return_property` は 4.4 ms なので、残りの約 4 ms が並べ替えのぶん。

計測は task182 のベンチ（`bash scripts/run_bench_local.sh`、2026-11-16、10 コア）。

## 進め方

- どこで時間を使っているかを先に確かめる（全行の RETURN 式を評価してから並べ替えている可能性）
- 上位 N 件だけを保つ（ヒープ）。並べ替えキーだけ先に評価し、RETURN の式は残った行だけ評価する

## 完了の目安

- `read/sort/order_by_limit` が `scan/return_property` より短くなる
- 変更の前後を `scripts/perf_check.py` で比べ、他の項目が遅くなっていないこと

## 結果（2026-11-16）

上位 N 件の選択（`select_nth_unstable_by`）はもとからあった。遅かった理由は 2 つ。

1. 全行の RETURN の式を評価してから選んでいた → `ORDER BY … LIMIT`（DISTINCT なし）では
   並べ替えキーだけを全行で求め、残る行だけ RETURN の式を評価する（`build_top_rows`）
2. ORDER BY のキーが返却列のどれに当たるかを、行ごとに列名の文字列を作って探していた
   → 文ごとに 1 回だけ求める（`order_key_slots`）。LIMIT の無い並べ替えと WITH・集計後の並べ替えにも効く

同じキーの行は入力の順に並ぶ（全件を並べてから切った結果と同じ）。
残らない行の RETURN の式は評価しないので、その行でだけ起きる実行時エラーは出なくなる。

| 項目 | 前 | 後 |
|---|---:|---:|
| read/sort/order_by_limit | 8.61 ms | 4.73 ms |
| read/sort/order_by_all | 8.90 ms | 7.73 ms |

残りの約 4.2 ms は並べ替え以外（スキャンと行の組み立て、`read/scan/return_property` と同じ）で、task188 の範囲。
