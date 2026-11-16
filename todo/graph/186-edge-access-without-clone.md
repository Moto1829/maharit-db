# 186: エッジを辿るときに複製しない・種別で絞る

## 問題

`outgoing_edges` / `incoming_edges` は `Vec<Edge>` を返し、エッジごとにラベルの `String` を複製する。種別の絞り込みは複製した後。
`read/hop/rare_type_on_hub`（10,010 本から 10 本）が 788 µs、`read/hop/all_1`（50,000 行）が 24.8 ms。

計測は task182 のベンチ（`bash scripts/run_bench_local.sh`、2026-11-16、10 コア）。

## 進め方

- `GraphBackend` に、複製せずに走査する方法（コールバックか ID + 種別だけを返す）を足す
- ノードごとの隣接リストをエッジ種別で分ける（`-[:TYPE]->` で対象の種別だけを見る）
- ACL のラッパー・記録用のラッパー（`RecordingGraph`）も合わせる

## 完了の目安

- `read/hop/rare_type_on_hub` が 10 本ぶんの時間になり、`read/hop/all_1` が短くなる
- 変更の前後を `scripts/perf_check.py` で比べ、他の項目が遅くなっていないこと
