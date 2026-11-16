# 184: ACL のあるセッションでもプロパティ索引を使う

## 問題

ACL の規則が 1 つでもあるセッションは、プロパティ索引での絞り込みを一切使わない（`index_lookup_allowed`）。
`read_acl/filter/eq_indexed` は 3.07 ms で、ACL なし（2.0 µs）の約 1,500 倍。

計測は task182 のベンチ（`bash scripts/run_bench_local.sh`、2026-11-16、10 コア）。

## 進め方

- 索引のラベル・プロパティがそのセッションで読める場合は索引を使う（読めないプロパティの索引だけ使わない）
- 範囲索引（range hints）も同じ条件で使う
- 見えないノードが候補に入っても `node_matches_pattern` が ACL のラッパー越しに確かめることをテストで固定する

## 完了の目安

- `read_acl/filter/eq_indexed` が ACL なしと同じ桁になる
- 変更の前後を `scripts/perf_check.py` で比べ、他の項目が遅くなっていないこと

## 結果（2026-11-16）

索引の結果は候補でしかなく、`node_matches_pattern` がラベルとプロパティの値を
`graph_ref()`（ACL 適用中は ACL のラッパー）越しに確かめ直している。そのため、読めないプロパティの
索引も含めて、ACL 適用中に索引を使っても結果は変わらない。`index_lookup_allowed` を削除した。

- `read_acl/filter/eq_indexed`: 3.07 ms → 2.5 µs
- テスト: `acl::tests::index_lookups_respect_read_rules`（読めないラベル・複数ラベル・読めないプロパティの等値と範囲）
