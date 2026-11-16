# 187: パスを絞り込める側の端から辿る

## 問題

パスは常に左端のノードから展開する。`MATCH (p:Person)-[:LIVES_IN]->(c:City {name: 'City7'})` は City 1 件から辿れば 100 本で済むが、Person 10,000 件から辿って 15.6 ms（`read/hop/selective_end`）。
プランナー（`planner.rs`）は EXPLAIN 用で、実行には使われていない。

計測は task182 のベンチ（`bash scripts/run_bench_local.sh`、2026-11-16、10 コア）。

## 進め方

- 両端の候補数（束縛済み・索引の等値・ラベルの件数）を見積もり、少ない側から逆向きに辿る
- 行の順序が変わる。ORDER BY の無いクエリの順序は保証していないが、テストと docs の出力例への影響を確かめる
- EXPLAIN の表示を実際の実行順に合わせる

## 完了の目安

- `read/hop/selective_end` が 100 本ぶんの時間になる
- 変更の前後を `scripts/perf_check.py` で比べ、他の項目が遅くなっていないこと
