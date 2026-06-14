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
