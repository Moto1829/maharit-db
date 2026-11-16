# 183: 索引で引く検索でも、ラベルの全ノード ID を作っていた

## 問題

`match_node_pattern` は最初に「スキャン用の候補」（ラベルの全ノード ID）を作っていた。
プロパティ索引で引く場合や、変数が束縛済みの場合はこの候補を使わないのに、
10,000 ノードのラベルで毎回約 60 µs かかり、1 件検索・1 点からのホップ・`SET`・`MERGE` の下限になっていた。

## 対応

候補を最初に使うときに作るようにした（`OnceCell`）。結果は変わらない。

## 計測（エンジン単体、Person 10,000 件、task182 のベンチ）

| 項目 | 前 | 後 |
|---|---:|---:|
| read/filter/eq_indexed | 60.4 µs | 2.0 µs |
| read/hop/point_1 | 63.7 µs | 4.3 µs |
| read/hop/point_2 | 77.5 µs | 18.1 µs |
| read/hop/point_3 | 167 µs | 109 µs |
| read/path/var_length_1_3 | 178 µs | 115 µs |
| write/set_property | 61.4 µs | 2.1 µs |
| write/merge_existing | 59.3 µs | 1.0 µs |
| write/create_delete_edge | 121 µs | 4.5 µs |

変わらなかったもの: `read/scan/count`（61 µs、件数を数えるのに ID の一覧を作っている）、
`read/filter/range_indexed`（69 µs）、`read_acl/filter/eq_indexed`（3.07 ms、task184）。
