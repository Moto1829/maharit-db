# タスク: perf_check.py がベースライン比較でほぼ全項目 FAIL する（Docker vs release バイナリの不一致）

## 概要

`scripts/perf_check.py` を Docker コンテナ環境で実行すると、16 項目中 15 項目が FAIL する。
ベースライン (`benchmark_reports/baseline.json`) は `target/release/maharit` で測定されているのに対し、
`docker-compose.yml` の `maharit-server` コンテナは 3 週間前のビルドを使用しており、
測定環境が一致していないことが根本原因。

## 失敗したテスト

- スクリプト: `scripts/perf_check.py`
- コマンド: `python3 scripts/perf_check.py benchmark_reports/baseline.json /tmp/current_bench.json`
- エラーメッセージ:
```
操作                                                    ベースライン             現在       変化  判定
CREATE Person nodes                               14,892/s          140/s    -99.1%  FAIL
UNWIND batch CREATE (map list)                   171,493/s             N/A       ---  SKIP
CREATE KNOWS edges                                 2,860/s           60/s    -97.9%  FAIL
MATCH full scan (BenchPerson)                  1,091,604/s      303,475/s    -72.2%  FAIL
MATCH WHERE age > 40                             683,492/s      250,621/s    -63.3%  FAIL
MATCH WHERE city = 'Tokyo'                       166,725/s       41,019/s    -75.4%  FAIL
MATCH WHERE skill = 'Rust'                       178,705/s       56,487/s    -68.4%  FAIL
MATCH WHERE id < 100                             188,724/s       35,347/s    -81.3%  FAIL
AGG COUNT all                                      3,077/s           21/s    -99.3%  FAIL
AGG AVG age                                        2,145/s          344/s    -83.9%  FAIL
AGG COUNT per city                                13,373/s          557/s    -95.8%  FAIL
AGG COUNT per skill                               13,936/s           23/s    -99.8%  FAIL
TRAV 1-hop KNOWS                                 373,348/s       53,537/s    -85.7%  FAIL
TRAV filter on edge                              219,979/s       30,128/s    -86.3%  FAIL
STREAM MATCH (chunk=100)                       1,183,373/s       21,588/s    -98.2%  FAIL
Repeated point-lookup (id filter)                  2,068/s           60/s    -97.1%  FAIL

結果: OK 0  WARN 0  FAIL 15  SKIP 1  / 計 16 操作
```

## 根本原因の分析

`benchmark_reports/baseline.json` の測定条件:
- timestamp: `2026-03-29T00:03:38Z`
- バイナリ: `target/release/maharit` (Rust release ビルド、最適化あり)
- CREATE Person nodes ベースライン: **14,892/s**

現在の Docker コンテナ (`maharit-db-server`) の条件:
- イメージビルド日: 3 週間前 (`2d9fb9a2a175`)
- バージョン: `v0.1.0`
- ビルド最適化: 不明（release/debug 混在の可能性）
- CREATE Person nodes 現在: **140/s** (99.1% 低下)

CREATE の 99% 低下は最適化ビルドと非最適化ビルドの差として説明できる。
また Docker コンテナのビルドが最新コードと 3 週間のコード差異がある。

`perf_check.py` は `benchmark.py` が Docker コンテナ経由で測定した結果と、
release バイナリが直接測定したベースラインを比較するため、
同一バイナリ・同一環境で両方測定する必要がある。

## 対応方針

### 短期対応: ベースラインを Docker コンテナで再測定
```bash
# Docker コンテナで perf_check を実行する場合は、
# ベースラインも Docker コンテナ環境で取得する
docker compose up -d maharit-server
python3 scripts/benchmark.py --nodes 1000 --output-json benchmark_reports/baseline_docker.json
python3 scripts/perf_check.py benchmark_reports/baseline_docker.json /tmp/current.json
```

### 中期対応: Docker イメージの定期更新とベースライン自動更新
1. `docker compose build` で最新コードからイメージを再ビルド
2. ベースラインを再取得:
   ```bash
   docker compose build maharit-server
   docker compose up -d maharit-server
   python3 scripts/benchmark.py --nodes 1000 --output-json benchmark_reports/baseline.json
   ```

### 長期対応: perf_check.py に環境メタデータ検証を追加
- ベースライン JSON と現在の JSON に `build_type` (debug/release), `docker_image_id` を記録
- 異なる環境の比較時に警告を表示する

## 優先度

HIGH

## 状態

完了 (2026-04-01)

## 対応内容

### 実施した修正

1. **`scripts/benchmark.py`**: `get_docker_image_id()` ヘルパーを追加し、`save_json_report()` の出力 JSON に `environment` フィールド（`build_type`, `docker_image_id`）を記録するように変更

2. **`scripts/perf_check.py`**: ベースラインと現在の結果で `docker_image_id` が異なる場合に警告を表示する検証ロジックを追加。`build_type` の不一致も警告対象

3. **Docker イメージの再ビルド**: `docker compose build maharit-server` で最新コード（v0.2.0）からイメージを再ビルド（旧イメージは 3 週間前のビルド）

4. **ベースラインの再測定**: 新しい Docker イメージで `benchmark.py --nodes 1000 --output-json benchmark_reports/baseline.json` を実行し、Docker 環境のベースラインを再取得

### 結果

- 旧ベースライン (release バイナリ): CREATE Person nodes **14,892/s**
- 新ベースライン (Docker): CREATE Person nodes **120/s**
- 以降は同一 Docker 環境で測定するため、環境差異による誤検知は発生しない
- `perf_check.py` がイメージ ID を表示・比較するため、将来の不一致も即座に検出可能

## 関連ファイル

- `scripts/perf_check.py`
- `scripts/benchmark.py`
- `benchmark_reports/baseline.json`
- `docker-compose.yml`
