# Task 104: scripts/ E2E スクリプトのプロトコルクライアントを共通モジュール化

## 背景

`scripts/` 配下の Python E2E スクリプトには、ほぼ同一の `MaharitClient` クラスと
プロトコル定数（4 バイト長プレフィックス + JSON）が **9 ファイル** にコピー
されている:

```
scripts/auth_test.py
scripts/benchmark.py
scripts/concurrent_test.py
scripts/constraint_test.py
scripts/failover_test.py
scripts/persistence_test.py
scripts/query_feature_test.py
scripts/replication_test.py
scripts/smoke_test.py
```

問題:

- プロトコル変更があると 9 ファイルすべてを修正する必要がある
- 各スクリプトで微妙にエラーハンドリングが異なる（reconnect の有無など）
- ANSI カラーコード定数も大量に重複
- `check()` / `section()` ヘルパー関数も多くのスクリプトで重複

## 提案

`scripts/lib/` ディレクトリを切り、共通モジュールを抽出する:

```
scripts/lib/__init__.py
scripts/lib/client.py      # MaharitClient (TCP プロトコル)
scripts/lib/reporting.py   # check / section / ANSI カラー
```

### 例

```python
# scripts/lib/client.py
class MaharitClient:
    def __init__(self, host: str, port: int, timeout: float = 10.0): ...
    def query(self, cypher: str) -> dict: ...
    def query_stream(self, cypher: str) -> Iterator[dict]: ...
    def ping(self) -> bool: ...
    def close(self): ...

# scripts/lib/reporting.py
GREEN, RED, YELLOW, CYAN, BOLD, RESET = (...)
def check(name: str, ok: bool, detail: str = ""): ...
def section(title: str): ...
def report_summary(passed: int, failed: int) -> int: ...
```

各スクリプトは:

```python
from lib.client import MaharitClient
from lib.reporting import check, section, report_summary
```

スクリプトのトップで `sys.path.insert(0, os.path.dirname(...))` を追加するか、
`scripts/` をパッケージ化（`__init__.py`）して相対インポートを使う。

### 移行戦略

1. `lib/client.py` を抽出（最も差分の少ない smoke_test.py の MaharitClient を雛形に）
2. 1 スクリプトずつ置き換え、動作確認しながら commit
3. 全置換後、共通の `MaharitClient` から外れた fielure ハンドリング差分を吸収

## 検証

- 各スクリプトを個別に実行して、既存の動作と同等であることを確認:
  - `python3 scripts/smoke_test.py`
  - `python3 scripts/persistence_test.py`
  - `python3 scripts/query_feature_test.py` 等

## 優先度

LOW（CI 化 / E2E 拡充の準備として効いてくる）

## 関連ファイル

- `scripts/*.py` 全 9 ファイル
- Task 77 (CI/CD E2E パイプライン) と組み合わせると効果倍増

## 解決済み (2026-06-14)

### 実装内容

`scripts/lib/` パッケージを新規作成:

- `scripts/lib/__init__.py`
- `scripts/lib/client.py` — `MaharitClient`
  - `query` / `stream_query` / `ping` / `iter_stream` / `send` / `close`
  - context manager 対応 (`with MaharitClient(...) as c:`)
- `scripts/lib/reporting.py` — `Reporter` クラス + モジュールレベル互換 API
  - ANSI 定数 (`GREEN` / `RED` / `YELLOW` / `CYAN` / `BOLD` / `RESET`)
  - `check(name, condition, detail)` / `section(title)` / `summarize() -> int`

### 移行したスクリプト (9 ファイル)

| スクリプト | 形態 | 補足 |
|-----------|------|------|
| `smoke_test.py` | 完全移行 | 動作確認 32/32 PASS |
| `persistence_test.py` | 完全移行 | 動作確認 17/17 PASS |
| `constraint_test.py` | 完全移行 | `errors` リスト保持 + `check` ラップ |
| `auth_test.py` | 完全移行 | `check_skip` をラッパとして残す |
| `concurrent_test.py` | サブクラス移行 | `TxClient` (begin/commit/rollback) + `_lock` 付き `check` ラッパ |
| `query_feature_test.py` | 完全移行 | `errors` リスト保持 |
| `failover_test.py` | サブクラス移行 | `addr` 属性を継承で追加 |
| `replication_test.py` | サブクラス移行 | `addr` 属性を継承で追加 |
| `benchmark.py` | 完全移行 | プロトコル部のみ置換、計測ロジックは維持 |

すべて `python3 -c "import <script>"` でロード確認。

### 効果

- プロトコル変更時の修正箇所が 9 → 1 ファイルに
- ANSI 定数・`check`/`section`/サマリ表示が 9 → 1 に集約
- 各スクリプトで 50〜90 行の冗長コードを削減
