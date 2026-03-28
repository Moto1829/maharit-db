# タスク: パフォーマンス回帰チェックのベースライン更新

## 概要

`scripts/perf_check.py` を `benchmark_reports/baseline.json` と比較すると、
6項目が FAIL（20% 以上の劣化）として検出される。しかし `baseline.json` は
Task 81 で手動で設定した仮の数値であり、実際の計測値ではない。

現在の `debug` ビルドで実際に計測した場合、ベースライン値と大きく乖離しているため、
正確な回帰検知には実際のサーバー計測に基づくベースラインへの更新が必要。

## 失敗したテスト

- スクリプト: `scripts/perf_check.py`
- エラーメッセージ:
```
回帰検出 — 以下の操作が 20% 以上劣化しました:
  ✗ MATCH WHERE age > 40
      ベースライン: 300,000/s  →  現在: 141,083/s  (53.0% 劣化)
  ✗ MATCH WHERE city = 'Tokyo'
      ベースライン: 120,000/s  →  現在: 35,041/s  (70.8% 劣化)
  ✗ MATCH WHERE skill = 'Rust'
      ベースライン: 100,000/s  →  現在: 35,500/s  (64.5% 劣化)
  ✗ TRAV 1-hop KNOWS
      ベースライン: 250,000/s  →  現在: 71,313/s  (71.5% 劣化)
  ✗ TRAV filter on edge
      ベースライン: 150,000/s  →  現在: 43,110/s  (71.3% 劣化)
  ✗ STREAM MATCH (chunk=100)
      ベースライン: 400,000/s  →  現在: 213,879/s  (46.5% 劣化)
```

また `UNWIND batch CREATE (map list)` は Task 88 のバグにより SKIP 扱いになっている。

## 根本原因の分析

`benchmark_reports/baseline.json` は Task 81 のコミット
(e0101deb) 時点で手動設定された推定値（1ノード=10ms想定など）で作成されており、
実際のサーバー計測値ではない。

現在の debug ビルドで実際に計測した値（2026-03-29 時点）:
- MATCH WHERE age > 40: 141,083/s
- TRAV 1-hop KNOWS: 71,313/s
- STREAM MATCH (chunk=100): 213,879/s

これらは十分に高いスループットであり、実際には回帰していない可能性が高い。

## 対応方針

1. `release` ビルドで実際のベンチマークを実行する:
   ```bash
   cargo build --release -p maharit-server
   ./target/release/maharit server --port 7687 --data /tmp/bench.db &
   python3 scripts/benchmark.py --nodes 1000 --output-json benchmark_reports/baseline.json
   ```
2. 実計測に基づいた `benchmark_reports/baseline.json` をコミットし直す
3. Task 88 (UNWIND マップリストバグ) を修正してから再計測する
4. `perf_check.py` が実際の回帰を検知できるようにする

## 優先度

MEDIUM

## 関連ファイル

- `benchmark_reports/baseline.json`
- `scripts/benchmark.py`
- `scripts/perf_check.py`
