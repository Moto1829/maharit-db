# 182: 性能改善の前に、足りないベンチマークを取れるようにする

## 背景

性能改善の候補（行の束縛の複製、エッジの複製、クエリの実行スレッド、読み書きのロック、
パターンの開始点、応答の JSON、ACL 有効時の索引 など）を挙げたが、効果を測る手段が足りなかった。

- `benchmark_reports/baseline.json` は 2026-07 のもので、WAL・文ごとの undo・ACL が入る前
- エンジン単体のベンチが無く、`scripts/benchmark.py` はクライアントと JSON の時間が混ざる
- 複数接続のスループットと、負荷がある間の応答時間を測る項目が無い

## やったこと

- `crates/maharit-query/benches/engine.rs`（criterion）: エンジン単体のベンチ 36 項目
  - parse / スキャン / 絞り込み（索引あり・なし・式）/ 1〜3 ホップ / 種別の違うエッジが多いノード /
    右端が絞り込めるパターン / 可変長・最短経路 / 集計・並べ替え / ACL あり / 応答の JSON 化 / 書き込み
  - 各クエリの行数を実行前に確かめる（空振りを測らないため）
- `scripts/benchmark.py`: 2 ホップ、複数接続のスループット（`PARALLEL`）、
  重いクエリ・書き込みが走っている間の軽いクエリの p50 / p99（`LAT`）を追加。`--parallel` / `--no-parallel`
- `scripts/engine_bench_report.py`: criterion の結果を表と JSON（`perf_check.py` で比べられる形）にする
- `scripts/run_bench_local.sh`: 上の 2 つをサーバーの起動からまとめて実行する

## 2026-11-16 の計測（main 9dd4b716、Apple Silicon 10 コア、ローカル）

エンジン単体（Person 10,000 / KNOWS 50,000）:

| 項目 | 時間 | 分かったこと |
|---|---:|---|
| read/filter/eq_indexed | 60 µs | 索引で 1 件を引くだけなのに `scan/count`（60 µs）と同じ。索引を使う場合もラベルの全ノード ID を先に作っている（`match_node_pattern` の `all_node_ids`） |
| read/hop/point_1 / set_property / merge_existing | 59〜64 µs | 同じ理由で 60 µs が下限になっている |
| read_acl/filter/eq_indexed | 3.08 ms | ACL のあるセッションは索引を使わないので 51 倍 |
| read/hop/all_1 | 24.8 ms | 50,000 行で 1 行あたり約 500 ns |
| read/hop/all_2 | 146 ms | 250,000 行 |
| read/hop/rare_type_on_hub | 788 µs | 10,010 本のうち 10 本を選ぶのに全エッジを複製して調べる |
| read/hop/selective_end | 15.6 ms | City 1 件から辿れば 100 本で済むが、Person 10,000 件の側から辿る |
| read/scan/return_property | 4.40 ms | プロパティ 1 つを返すだけで 1 行 440 ns |
| read/sort/order_by_limit | 8.61 ms | `LIMIT 10` を付けても全件の並べ替え（8.90 ms）とほぼ同じ |
| response/json_10k_properties | 4.77 ms | 応答の JSON 化はクエリの実行（4.4 ms）と同じくらいかかる |
| write/create_node | 2.66 µs | エンジンだけなら 1 件 2.7 µs |

サーバー（1,000 ノード、WAL あり）:

| 項目 | 結果 | 分かったこと |
|---|---:|---|
| PARALLEL CREATE x1 | 330 /s | 1 件 3 ms。エンジンは 2.7 µs なので、ほぼすべて WAL の fsync 待ち |
| PARALLEL CREATE x8 | 948 /s | 8 接続で 2.9 倍 |
| PARALLEL point-lookup x1 → x8 | 3,398 → 11,000 /s | 8 接続で 3.2 倍 |
| LAT light (idle) p50 / p99 | 0.30 / 0.44 ms | |
| LAT light (heavy readers = cores) p50 / p99 | 41.8 / 1,781 ms | 重いクエリがコア数ぶん走ると、軽いクエリが重いクエリの終了まで待たされる |
| LAT light (2 batch writers) p50 / p99 | 2.23 / 5.01 ms | 書き込み中は読み取りが待つ |

## 残り

- 改善タスクはこの計測をもとに別に起こす
- `benchmark_reports/baseline.json`（Docker、2026-07）は更新していない
