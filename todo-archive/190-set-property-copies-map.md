# 190: プロパティを 1 つ書くたびにマップ全体を複製している

## 問題

`ConcurrentGraph::set_node_property` / `set_edge_property` は `(*node.properties).clone()` で毎回マップ全体を複製する。プロパティが k 個のノードを作ると k 回複製する。
`write/set_indexed_property_all`（10,000 件）が 28.6 ms、`write/unwind_create_1000` が 1.72 ms。

計測は task182 のベンチ（`bash scripts/run_bench_local.sh`、2026-11-16、10 コア）。

## 進め方

- `Arc::make_mut` を使い、他に参照が無いときは複製しない
- 削除（`remove_*_property`）も同じ

## 完了の目安

- `write/unwind_create_1000` と `write/set_indexed_property_all` が短くなる
- 変更の前後を `scripts/perf_check.py` で比べ、他の項目が遅くなっていないこと

## 結果（2026-11-16）

`set_node_property` / `set_edge_property` を `Arc::make_mut` にした（削除はもとから `make_mut`）。
ベンチのノードはプロパティが 2〜4 個なので、効果は 1 割前後だった。プロパティが多いノードほど効く。

| 項目 | 前 | 後 |
|---|---:|---:|
| write/create_node | 2.66 µs | 2.28 µs |
| write/unwind_create_1000 | 1.72 ms | 1.57 ms |
| write/set_indexed_property_all | 28.6 ms | 25.9 ms |
