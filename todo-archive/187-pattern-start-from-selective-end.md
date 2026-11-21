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

## 結果（2026-11-21、完了）

- `match_path_pattern` が両端の候補数を見積もり、終点が始点の半分未満ならパターンを逆向きにして終点から辿る（`reversed_if_cheaper`）。
  見積もり: 束縛済み 1、索引のある等値 → 索引の件数、ほかのプロパティ条件 → ラベル件数の 1/10、それ以外はラベル件数（無ければ全ノード）。
  WHERE の等値はパターンに押し下げてから見積もるので、`WHERE c.name = 'x'` も効く
- 逆にするのは、パス変数・最短経路・可変長を含まない固定長のパスだけ（行の集合は同じで、順序だけが変わる）
- 見積もりは索引を引くので、始点が束縛済み・終点がラベルも束縛も無い（始点より少なくなりえない）ときは省く
- `GraphBackend::count_nodes_by_label` を追加（ID を集めずに件数だけ）
- EXPLAIN もプランナーで同じ規則を使い、逆向きのときは `NodeByLabelScan … (reordered)` + `ExpandReverse` を出す（複数ホップも）
- ORDER BY の無いクエリの行の順序が変わるが、テスト・docs の例・E2E は全て通った

エンジンのベンチ（変更前 → 後）: `read/hop/selective_end` 5.3 ms → 63 µs。他の `read/` の項目は ±2% 以内。
