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
