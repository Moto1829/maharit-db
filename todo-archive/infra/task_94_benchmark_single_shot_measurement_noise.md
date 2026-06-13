# タスク: benchmark.py の単発計測による perf_check FAIL（計測ノイズ問題）

## 概要

`scripts/perf_check.py` を同一 Docker イメージ・同一ノード数で実行しても、
フィルタクエリ・トラバーサル・ストリーム系の項目が 20%〜50% の劣化として検出され FAIL する。
根本原因は `scripts/benchmark.py` が高速クエリ（0.00〜0.04s）を**単発 1 回のみ実行**しているため、
OS スケジューラや TCP レイテンシのノイズが支配的になり計測精度が著しく低いこと。

## 失敗したテスト

- スクリプト: `scripts/perf_check.py`
- コマンド:
  ```
  docker compose up -d maharit-server
  python3 scripts/benchmark.py --nodes 1000 --output-json /tmp/run1.json
  python3 scripts/benchmark.py --nodes 1000 --output-json /tmp/run2.json
  python3 scripts/perf_check.py benchmark_reports/baseline.json /tmp/run1.json
  python3 scripts/perf_check.py benchmark_reports/baseline.json /tmp/run2.json
  ```

- Run1 と Run2 の比較（同一環境・同一イメージ、image_id: sha256:0467432e079d）:

```
操作                              ベースライン    Run1       Run2    変動率(R1/R2)
--------------------------------------------------------------------------------
MATCH full scan (BenchPerson)     116,371/s   238,471/s  112,020/s  0.47x  (Run1が異常高値)
MATCH WHERE age > 40              167,566/s   141,750/s  113,955/s  0.80x
MATCH WHERE city = 'Tokyo'         43,296/s    23,380/s   26,598/s  1.14x
MATCH WHERE skill = 'Rust'         53,191/s    26,580/s   30,909/s  1.16x
MATCH WHERE id < 100               53,402/s    29,144/s   40,467/s  1.39x
TRAV 1-hop KNOWS                   72,213/s    40,889/s   66,558/s  1.63x
TRAV filter on edge                60,480/s    32,024/s   56,392/s  1.76x
STREAM MATCH (chunk=100)          253,778/s   197,879/s  251,688/s  1.27x
```

- Run1 での perf_check 結果: FAIL 6件（city=Tokyo: -46%, skill=Rust: -50%, TRAV: -43〜-47%, STREAM: -22%）
- Run2 での perf_check 結果: FAIL 4件（age>40: -32%, city=Tokyo: -39%, skill=Rust: -42%, id<100: -24%）

## 根本原因の分析

`scripts/benchmark.py` の `bench_filter_queries()` 関数（line 195-216）は
各 WHERE フィルタクエリを **1 回のみ実行**して `elapsed` を計測している:

```python
start   = time.perf_counter()
resp    = client.query(cypher)          # ← 1回だけ
elapsed = time.perf_counter() - start
count   = len(resp.get("rows", []))
r = BenchResult(f"MATCH {label}", count, elapsed)
```

計測時間が 0.00〜0.04s の高速クエリでは:
- TCP ラウンドトリップ時間（通常 0.1〜1ms）が結果に大きく影響する
- OS スケジューラの割り込みで外れ値が発生しやすい
- 同一条件でも Run 間で 1.7x 以上の変動が起きる

同様の問題が `bench_traversal()`、`bench_stream()` にも存在する。

`perf_check.py` の失敗閾値 20% はこのノイズレベル（実測 0.47x〜1.76x）に対して過小である。

## 対応方針

### 方針 A: 繰り返し計測に変更（推奨）

高速クエリ（elapsed < 1.0s）は複数回実行して中央値または最小値で評価する:

```python
REPEAT_MIN_SECS = 1.0  # 合計実行時間がこの秒数を超えるまで繰り返す
WARMUP_ITERS = 3       # ウォームアップ回数（計測対象外）

# ウォームアップ
for _ in range(WARMUP_ITERS):
    client.query(cypher)

# 計測
times = []
deadline = time.perf_counter() + REPEAT_MIN_SECS
while time.perf_counter() < deadline:
    start = time.perf_counter()
    resp = client.query(cypher)
    times.append(time.perf_counter() - start)

elapsed = sorted(times)[len(times) // 2]  # 中央値
count = len(resp.get("rows", []))
```

### 方針 B: perf_check.py の閾値を緩和

単発計測を維持するなら WARN 閾値 30%、FAIL 閾値 50% 程度が適切。
ただし本物の回帰を見逃すリスクが高まるため推奨しない。

### 方針 C: 両方対応（最善）

1. `benchmark.py` で高速クエリの繰り返し計測を実装
2. `perf_check.py` に `--threshold` デフォルト値の見直し（現 0.20 → 0.30）
3. ベースラインを繰り返し計測版で再取得

## 影響範囲

`bench_filter_queries()`, `bench_traversal()`, `bench_stream()` の全クエリ。
`bench_create_nodes()` と `bench_create_edges()` は元々秒単位の計測なので影響なし。

## 優先度

MEDIUM

## 状態

完了 (2026-04-06)

## 対応内容

### 方針C（両方対応）を実施

1. **`scripts/benchmark.py`** に繰り返し計測ヘルパーを追加:
   - `REPEAT_MIN_SECS = 1.0`、`WARMUP_ITERS = 3` 定数を定義
   - `_run_timed()`: 単一クエリを 1 秒以上繰り返し、中央値レイテンシを返す
   - `_run_timed_stream()`: streamQuery を 1 秒以上繰り返し、中央値レイテンシを返す
   - `bench_full_scan`、`bench_filter_queries`、`bench_aggregation`、`bench_traversal`、`bench_stream` を繰り返し計測方式に変更

2. **`scripts/perf_check.py`** のデフォルト閾値を `0.20 → 0.30` に変更（方針C）

3. **ベースラインを繰り返し計測版で再取得**

### 結果

| | 修正前 | 修正後 |
|--|--------|--------|
| 同一環境 2回計測での FAIL | 4〜6件 | 0件 |
| 計測のばらつき（TRAV系） | 1.63x〜1.76x | ±3% 以内 |
| MATCH WHERE 系スループット | 24,000〜41,000/s | 100,000〜413,000/s（中央値で安定） |

## 関連ファイル

- `scripts/benchmark.py` (line 195-216: bench_filter_queries, line 250+: bench_traversal, bench_stream)
- `scripts/perf_check.py`
- `benchmark_reports/baseline.json`
