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

## 結果（2026-11-17、完了）

- `GraphBackend::adjacent_edges(node, outgoing, types, out)` を追加。エッジの ID と両端（`EdgeEnds`）だけを返し、種別名やプロパティを複製しない。ACL のラッパーは相手側が見えないものを除き、`RecordingGraph` はそのまま委ねる
- `ConcurrentGraph` の隣接リストを種別ごとに分け、相手側のノード ID も持たせた（辿るときにエッジ本体を引かない）。種別名は共有（`Arc<str>`）
- エッジのプロパティは、パターンにプロパティ条件があるときだけ取りに行く

エンジンのベンチ（10,000 Person、変更前 → 後）:

| 項目 | 前 | 後 |
|---|---|---|
| read/hop/rare_type_on_hub | 906 µs | 5.4 µs |
| read/hop/all_1 | 31.0 ms | 16.6 ms |
| read/hop/all_2 | 163 ms | 96 ms |
| read/hop/all_1_where_both_ends | 56.4 ms | 38.5 ms |
| read/hop/selective_end | 17.7 ms | 5.0 ms |
| read/path/shortest | 7.0 ms | 3.7 ms |
| read_acl/hop/all_1 | 46.0 ms | 36.3 ms |
| write/create_path | 3.73 µs | 3.99 µs（+6%、ノードごとの種別の表を作る分） |

その他の項目は誤差の範囲（同じ項目を 2 回測ると ±10% ほど揺れる）。
