# 188: 行の束縛を HashMap から番号付きの配列にする

## 問題

`Bindings = HashMap<Arc<str>, BindingValue>` を 1 ホップ・1 行ごとに複製し、変数名の `Arc<str>` も束縛のたびに新しく確保している。
`read/hop/all_1` は 1 行あたり約 500 ns、`read/hop/all_2`（250,000 行）は 146 ms、`read/scan/return_property` は 1 行 440 ns。

計測は task182 のベンチ（`bash scripts/run_bench_local.sh`、2026-11-16、10 コア）。

## 進め方

- 文ごとに変数へ番号を割り当て、行を `Vec<BindingValue>`（または小さい固定長）にする
- まず変数名の `Arc<str>` を文ごとに 1 回だけ作る、という小さい段階から始めて効果を測る
- 影響範囲が広い（エグゼキュータ全体）ので、段階ごとにベンチと全テストで確かめる

## 完了の目安

- `read/hop/all_1` / `all_2` / `scan/return_property` の 1 行あたりの時間が半分以下になる
- 変更の前後を `scripts/perf_check.py` で比べ、他の項目が遅くなっていないこと
