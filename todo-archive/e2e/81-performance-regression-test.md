# Task 81: パフォーマンス回帰テスト

## 背景・目的

`benchmark.py` はベンチマークを実行してレポートを生成するが、
「前回より遅くなったら警告する」仕組みがない。

パフォーマンスの回帰（性能劣化）はコードレビューでは気づきにくく、
自動的な閾値チェックが必要。

## 実装内容

### ベースライン記録

```bash
# 現在の性能を baseline として記録
python3 scripts/benchmark.py --nodes 1000 --output benchmark_reports/baseline.json
```

JSON フォーマットで各操作のレイテンシ（p50/p95/p99）とスループットを保存。

### 回帰チェックスクリプト: `scripts/perf_check.py`

```python
def check_regression(baseline_path, current_path, threshold=0.2):
    """
    各操作のスループットが baseline の (1 - threshold) 倍を下回ったら FAIL
    threshold=0.2 → 20% 以上の劣化で失敗
    """
```

#### チェック項目

| 操作 | 失敗閾値 |
|------|---------|
| ノード作成（バルク） | ベースラインの 80% 未満 |
| MATCH（全件） | ベースラインの 80% 未満 |
| MATCH（フィルタ） | ベースラインの 80% 未満 |
| インデックス検索 | ベースラインの 80% 未満 |

### CI への組み込み（Task 77 と連携）

```yaml
- name: Run benchmark
  run: python3 scripts/benchmark.py --nodes 1000 --output /tmp/bench_current.json

- name: Check regression
  run: python3 scripts/perf_check.py baseline.json /tmp/bench_current.json
```

ただし CI 環境はハードウェアが一定でないため、回帰チェックは
`push to main` のみで実行し、PR では参考値として表示する。

### benchmark.py の改修

現在の `benchmark.py` は Markdown のみ出力するため、
JSON 出力オプション（`--output-json`）を追加する。

```json
{
  "timestamp": "2026-03-24T12:00:00Z",
  "node_count": 1000,
  "results": {
    "create_nodes": {"ops_per_sec": 5000, "p50_ms": 0.2},
    "match_all": {"ops_per_sec": 200, "p50_ms": 5.0}
  }
}
```

## 完了条件

- [x] `benchmark.py` に JSON 出力オプションが追加されていること
- [x] `perf_check.py` でベースライン比較ができること
- [x] 20% 以上の劣化で終了コード 1 を返すこと
- [x] ベースライン JSON がリポジトリに保存されていること
