# タスク: レプリケーション・フェイルオーバーテストのローカル実行環境整備

## 概要

`scripts/replication_test.py` と `scripts/failover_test.py` は Docker Compose で
リーダー1台 + フォロワー2台を起動した環境を前提としているが、
Docker 環境なしではフォロワーへの接続が全て失敗してテストが FAIL になる。
ローカルでの開発時に気軽に実行できるよう、単一マシン上でも検証できる環境整備が必要。

## 失敗したテスト

### replication_test.py

- スクリプト: `scripts/replication_test.py`
- エラーメッセージ:
```
  接続失敗 フォロワー1 (localhost:7689) — [Errno 61] Connection refused
  接続失敗 フォロワー2 (localhost:7690) — [Errno 61] Connection refused
  ...
  ✗ フォロワー1: 接続済み — 接続できていない
  ✗ フォロワー2: 接続済み — 接続できていない
  ...
結果: 12 passed, 4 failed
EXIT: 1
```

### failover_test.py

- スクリプト: `scripts/failover_test.py`
- エラーメッセージ:
```
  接続失敗 フォロワー1 (127.0.0.1:7689) — [Errno 61] Connection refused
  接続失敗 フォロワー2 (127.0.0.1:7690) — [Errno 61] Connection refused
  ...
  ✗ フォロワー1: 伝播確認 — 接続なし
  ✗ フォロワー2: 伝播確認 — 接続なし
  ...
EOFError: EOF when reading a line  ← --no-docker モードで input() が呼ばれる
EXIT: 1
```

## 根本原因の分析

両スクリプトはレプリケーションクラスターが前提で、単体サーバーのみの環境では
フォロワーに相当するプロセスが存在しない。

また `failover_test.py` の `--no-docker` モードは `input()` を使って手動操作を促すため、
CI や非インタラクティブ環境では `EOFError` が発生する。

## 対応方針

### オプション1: ローカル起動スクリプトを追加

`scripts/start_replication_cluster_local.sh` を作成し、
単一マシン上で異なるポートを使ってリーダー1台 + フォロワー2台を起動する:
```bash
./target/debug/maharit server --port 7687 --data /tmp/leader.db --role leader &
./target/debug/maharit server --port 7689 --data /tmp/follower1.db \
    --role follower --leader-addr 127.0.0.1:7687 &
./target/debug/maharit server --port 7690 --data /tmp/follower2.db \
    --role follower --leader-addr 127.0.0.1:7687 &
```

### オプション2: テストスクリプトに --skip-followers オプションを追加

接続できないフォロワーがいる場合に SKIP 扱いにする（現在は FAIL）ことで、
単体サーバー環境でもリーダー側のテストだけ実行できるようにする。

### オプション3: Docker Compose を CI のみで使用する前提にして、ローカルはスキップ

`replication_test.py` と `failover_test.py` に環境変数で実行をスキップする機能を追加:
```python
if os.environ.get("SKIP_REPLICATION_TESTS"):
    sys.exit(0)
```

## 優先度

MEDIUM

## 関連ファイル

- `scripts/replication_test.py`
- `scripts/failover_test.py`
- `docker-compose.replication.yml`
- `maharit-server/src/main.rs` (サーバー起動引数)
