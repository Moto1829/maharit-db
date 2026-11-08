---
title: サーバー設定
parent: サーバー・運用
nav_order: 1
---

# サーバー設定

## 永続化について

MaharitDB サーバーは **常にファイルベースの永続化** で動作します。SQLite と同様に、起動時にデータファイルのパスを指定します。

- ファイルが存在する場合: 既存データをロードして起動
- ファイルが存在しない場合: 新規作成して起動
- 書き込みクエリ: 変更を WAL（`<データファイル>.wal`）に追記してから応答する
- SIGINT / SIGTERM 受信時: データファイルを保存し、WAL を空にしてシャットダウン

> **注意**: サーバーはオンメモリ専用モードを持ちません。オンメモリで使いたい場合は `maharit-core` の `Graph::new()` を直接利用してください。

### WAL とクラッシュからの復旧

データファイル（スナップショット）はグラフ全体を保存したもので、停止時とチェックポイント時に書き換えます。その間の変更は、書き込みクエリごとに WAL へ追記されます。

起動時はデータファイルを読み込んだあと WAL を先頭から再適用するので、プロセスが強制終了（`kill -9`、OOM、電源断など）しても、**応答を返した書き込みは失われません**。最後のレコードが書きかけで終わっていた場合（応答前に落ちた書き込み）は、そのレコードを切り捨てて起動します。

`--wal` で、WAL の書き方を選べます。

| モード | 動作 | 守れる障害 |
|--------|------|-----------|
| `fsync`（デフォルト） | 追記して fsync してから応答する | プロセスの強制終了、OS のクラッシュ、電源断 |
| `os` | 追記するが fsync しない（OS のキャッシュに任せる） | プロセスの強制終了のみ |
| `off` | WAL を使わない（停止時とチェックポイント時にしか保存しない） | なし（前回の保存以降の変更が失われる） |

`fsync` モードでは、同時に届いた書き込みの fsync を 1 回にまとめます（グループコミット）。それでも書き込みの速さはディスクの fsync 性能で決まります。macOS（APFS）では、1 接続あたり毎秒数百件程度が目安です。大量に投入するときは、`UNWIND` で 1 クエリにまとめるか（1 クエリ = 1 回の fsync）、一時的に `--wal os` を使ってください。

**チェックポイント**: WAL が `--checkpoint-mb`（デフォルト 64MB）を超えると、書き込みを一時停止してデータファイルを保存し、WAL を空にします。データファイルの保存は一時ファイルに書いてから置き換えるので、保存中に落ちても壊れたファイルは残りません。

### バックアップを取るとき

`maharit backup --source <データファイル>` はデータファイルだけを読みます。稼働中のサーバーでは、前回のチェックポイント以降の変更は WAL にしかありません。漏れのないバックアップが必要なときは、サーバーを SIGTERM で停止して（データファイルが保存され、WAL が空になる）からバックアップを取ってください。

```bash
systemctl stop maharit          # または kill -TERM <pid>
maharit backup --source /var/lib/maharit/maharit.db --output /backup/maharit_$(date +%Y%m%d).db --compress
systemctl start maharit
```

!!! note "制限"
    - フォロワー（`--replication-role follower`）は WAL を書きません。フォロワーの変更はリーダーから届くため、停止時の保存とリーダーからの再同期に任せます。
    - トランザクション（`begin`）中に書いた変更は、実行した時点で WAL に書かれます。コミットもロールバックもしないうちにサーバーが落ちると、そのトランザクションの変更は再起動後に残ります（ロールバック済みのものは残りません）。
    - インデックス・制約・全文検索インデックスの定義は、データファイルにも WAL にも保存されません。再起動後に作り直してください。

## コマンドラインオプション

MaharitDB サーバーは以下のコマンドラインオプションで設定できます。一覧は `maharit server --help` でも確認できます。

```bash
maharit server [OPTIONS]
```

> **開発時の注意**: ビルドせずに起動する場合は `cargo run -p maharit-server -- server [OPTIONS]` を使用してください。

### 利用可能なオプション

| オプション | 短縮形 | デフォルト | 説明 |
|-----------|--------|-----------|------|
| `--host` | `-h` | `127.0.0.1` | バインドするホストアドレス |
| `--port` | `-p` | `7687` | リッスンするポート番号 |
| `--max-connections` | `-c` | `100` | 最大同時接続数 |
| `--data` | `-d` | `maharit.db` | データファイルのパス（環境変数: `MAHARIT_DATA`）。WAL は同じ場所の `<data>.wal` |
| `--wal` | | `fsync` | WAL のモード: `fsync` / `os` / `off`（環境変数: `MAHARIT_WAL`） |
| `--checkpoint-mb` | | `64` | WAL がこのサイズ（MB）を超えたらチェックポイントを取る。`0` で停止時のみ |
| `--require-auth` | | オフ | すべてのリクエストに有効なセッショントークンを要求する（[認証](auth.md)） |
| `--admin-password` | | なし | 初期管理者パスワード（環境変数: `MAHARIT_ADMIN_PASSWORD`）。`--require-auth` では必須 |
| `--tls-cert` / `--tls-key` | | なし | クライアント接続（リーダーではレプリケーション接続も）を TLS にする証明書と秘密鍵（PEM）。環境変数: `MAHARIT_TLS_CERT` / `MAHARIT_TLS_KEY`（[TLS/SSL 設定](tls.md)） |
| `--tls-min-version` | | `1.2` | 受け付ける最小の TLS バージョン: `1.2` / `1.3`（環境変数: `MAHARIT_TLS_MIN_VERSION`） |
| `--metrics-bind` | | `127.0.0.1:9090` | メトリクス・ヘルスチェック用 HTTP のアドレス（[メトリクス](metrics.md)） |
| `--replication-role` | | なし | `leader` / `follower`（[レプリケーション](../advanced/replication.md)） |
| `--replication-bind` | | `127.0.0.1:7688` | レプリケーションの待ち受けアドレス（リーダー） |
| `--replication-secret` | | なし | レプリケーション接続の共有シークレット（環境変数: `MAHARIT_REPLICATION_SECRET`） |
| `--leader-addr` | | なし | リーダーのレプリケーションアドレス（フォロワーで必須） |
| `--leader-ca` | | なし | リーダーの証明書を検証する CA 証明書。指定するとリーダーに TLS で接続する（フォロワー、環境変数: `MAHARIT_LEADER_CA`） |
| `--node-id` | | `node-1` | クラスター内で一意なノード ID（環境変数: `MAHARIT_NODE_ID`） |
| `--shard` / `--shard-id` | | なし | シャードノードとして起動（[シャーディング](../advanced/sharding.md)） |
| `--coordinator` / `--coordinator-port` / `--shards` / `--strategy` | | なし | コーディネーターとして起動 |

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
# /var/lib/maharit/maharit.db と /var/lib/maharit/maharit.db.wal を使う
maharit server --data /var/lib/maharit/maharit.db
```

### WAL の設定

```bash
# 大量投入の間だけ fsync を省く（プロセスの強制終了には耐える）
maharit server --wal os

# チェックポイントを 256MB ごとにする
maharit server --checkpoint-mb 256
```

### ログレベルの設定

ログの出力レベルは環境変数 `RUST_LOG` で指定します。

```bash
# デバッグログを有効化
RUST_LOG=debug maharit server

# 警告以上のみ
RUST_LOG=warn maharit server
```

### メトリクスの公開アドレス

```bash
maharit server --metrics-bind 0.0.0.0:9090
```

起動後、`http://localhost:9090/metrics` で Prometheus メトリクスが取得できます。

### TLS で待ち受ける

```bash
maharit server --host 0.0.0.0 \
  --tls-cert /etc/maharit/server.crt \
  --tls-key /etc/maharit/server.key
```

ネットワーク越しに使う場合は TLS を有効にしてください。TLS なしでループバック以外のアドレスに待ち受けると、起動時に警告が出ます。メトリクスの HTTP は TLS の対象外です（[TLS/SSL 設定](tls.md)）。

## 環境変数

コマンドラインオプションの代わりに環境変数でも設定できます。両方を指定した場合はオプションが優先されます。

| 環境変数 | 対応するオプション |
|---------|----------------|
| `MAHARIT_DATA` | `--data` |
| `MAHARIT_WAL` | `--wal` |
| `MAHARIT_ADMIN_PASSWORD` | `--admin-password` |
| `MAHARIT_REPLICATION_SECRET` | `--replication-secret` |
| `MAHARIT_NODE_ID` | `--node-id` |
| `MAHARIT_TLS_CERT` / `MAHARIT_TLS_KEY` | `--tls-cert` / `--tls-key` |
| `MAHARIT_TLS_MIN_VERSION` | `--tls-min-version` |
| `MAHARIT_LEADER_CA` | `--leader-ca` |
| `RUST_LOG` | ログレベル |

```bash
export MAHARIT_DATA=/var/lib/maharit/maharit.db
export MAHARIT_WAL=fsync
maharit server --host 0.0.0.0
```

## ログ出力

サーバーは JSON 形式の構造化ログを出力します。起動メッセージなど一部は平文でも出力されます。

```json
{"timestamp":"2026-11-08T05:14:29.548490Z","level":"INFO","fields":{"message":"Server listening","address":"0.0.0.0:7687"},"target":"maharit::tcp_server"}
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
Environment=RUST_LOG=info
ExecStart=/usr/local/bin/maharit server \
  --host 0.0.0.0 \
  --port 7687 \
  --data /var/lib/maharit/maharit.db
# 停止時にデータファイルを保存する時間を確保する
KillSignal=SIGTERM
TimeoutStopSec=60s
Restart=always
RestartSec=5s

[Install]
WantedBy=multi-user.target
```
