# サーバー設定

## 永続化について

MaharitDB サーバーは **常にファイルベースの永続化** で動作します。SQLite と同様に、起動時にデータファイルのパスを指定します。

- ファイルが存在する場合: 既存データをロードして起動
- ファイルが存在しない場合: 新規作成して起動
- SIGINT / SIGTERM 受信時: 自動でデータを保存してシャットダウン

> **注意**: サーバーはオンメモリ専用モードを持ちません。オンメモリで使いたい場合は `maharit-core` の `Graph::new()` を直接利用してください。

## コマンドラインオプション

MaharitDB サーバーは以下のコマンドラインオプションで設定できます。

```bash
maharit server [OPTIONS]
```

> **開発時の注意**: ビルドせずに起動する場合は `cargo run -p maharit-server -- server [OPTIONS]` を使用してください。

### 利用可能なオプション

| オプション | 短縮形 | 型 | デフォルト | 説明 |
|-----------|--------|-----|-----------|------|
| `--host` | `-H` | String | `127.0.0.1` | バインドするホストアドレス |
| `--port` | `-p` | u16 | `7687` | リッスンするポート番号 |
| `--max-connections` | `-c` | usize | `100` | 最大同時接続数 |
| `--data` | | Path | `maharit.db` | データファイルのパス（環境変数: `MAHARIT_DATA`） |
| `--log-level` | `-l` | String | `info` | ログレベル（trace/debug/info/warn/error） |
| `--tls-cert` | | Path | なし | TLS 証明書ファイルのパス |
| `--tls-key` | | Path | なし | TLS 秘密鍵ファイルのパス |
| `--metrics-port` | | u16 | `9090` | Prometheus メトリクスのポート |
| `--enable-replication` | | bool | `false` | レプリケーションを有効化 |
| `--replication-role` | | String | `leader` | レプリケーションのロール（leader/follower） |
| `--leader-addr` | | String | なし | フォロワー時のリーダーアドレス |

## 使用例

### 基本的な起動

```bash
# デフォルト設定（127.0.0.1:7687）
maharit server

# すべてのインタフェースにバインド
maharit server --host 0.0.0.0 --port 7687

# 最大接続数を設定
maharit server --host 0.0.0.0 --max-connections 500
```

### データファイルの指定

```bash
maharit server --data /var/lib/maharit/maharit.db
```

### ログレベルの設定

```bash
# デバッグログを有効化
maharit server --log-level debug

# エラーのみ
maharit server --log-level error
```

### TLS 付きで起動

```bash
maharit server \
  --host 0.0.0.0 \
  --port 7687 \
  --tls-cert /etc/maharit/server.crt \
  --tls-key /etc/maharit/server.key
```

### メトリクスポートの設定

```bash
maharit server --metrics-port 9090
```

起動後、`http://localhost:9090/metrics` で Prometheus メトリクスが取得できます。

## 環境変数

コマンドラインオプションの代わりに環境変数でも設定できます。

| 環境変数 | 対応するオプション |
|---------|----------------|
| `MAHARIT_HOST` | `--host` |
| `MAHARIT_PORT` | `--port` |
| `MAHARIT_MAX_CONNECTIONS` | `--max-connections` |
| `MAHARIT_DATA` | `--data` |
| `MAHARIT_LOG_LEVEL` | `--log-level` |
| `MAHARIT_TLS_CERT` | `--tls-cert` |
| `MAHARIT_TLS_KEY` | `--tls-key` |
| `MAHARIT_METRICS_PORT` | `--metrics-port` |

```bash
export MAHARIT_HOST=0.0.0.0
export MAHARIT_PORT=7687
export MAHARIT_DATA=/var/lib/maharit/maharit.db
maharit server
```

## ログ出力

サーバーは JSON 形式の構造化ログを標準出力に出力します。

```json
{"level":"INFO","message":"MaharitDB server starting","host":"0.0.0.0","port":7687,"timestamp":"2024-01-01T00:00:00Z"}
{"level":"INFO","message":"Server ready","connections":0,"timestamp":"2024-01-01T00:00:01Z"}
{"level":"INFO","message":"Client connected","peer":"192.168.1.100:54321","timestamp":"2024-01-01T00:01:00Z"}
{"level":"INFO","message":"Query executed","query":"MATCH (n:Person) RETURN n","duration_ms":2,"timestamp":"2024-01-01T00:01:00Z"}
```

ログをファイルにリダイレクトする場合：

```bash
maharit server > /var/log/maharit/server.log 2>&1
```

または `systemd` を使用する場合は `journald` で収集できます。

## systemd サービスの設定例

```ini
[Unit]
Description=MaharitDB Graph Database Server
After=network.target

[Service]
Type=simple
User=maharit
Group=maharit
ExecStart=/usr/local/bin/maharit server \
  --host 0.0.0.0 \
  --port 7687 \
  --data /var/lib/maharit/maharit.db \
  --log-level info
Restart=always
RestartSec=5s

[Install]
WantedBy=multi-user.target
```
