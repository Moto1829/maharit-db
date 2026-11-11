---
title: Docker での使用
parent: サーバー・運用
nav_order: 8
---

# Docker での使用方法

リポジトリの `Dockerfile` から、`maharit` を含むイメージを作れます。

## イメージを作る

```bash
docker build -t maharit-db .
```

イメージの中身:

- `/app/maharit`: バイナリ
- `/data`: データ用のボリューム（データファイルは `/data/maharit.db`、WAL は `/data/maharit.db.wal`）
- 既定のコマンド: `maharit server --host 0.0.0.0 --metrics-bind 0.0.0.0:9090`
- ポート: 7687（クライアント）、9090（[メトリクス・ヘルスチェック](metrics.md)）
- `HEALTHCHECK`: `http://localhost:9090/health` を 30 秒ごとに確認
- 実行ユーザー: `maharit`（root ではない）

## サーバーを起動する

```bash
docker run -d --name maharit \
  -p 7687:7687 -p 127.0.0.1:9090:9090 \
  -v maharit-data:/data \
  -e MAHARIT_ADMIN_PASSWORD="$MAHARIT_ADMIN_PASSWORD" \
  maharit-db server --host 0.0.0.0 --metrics-bind 0.0.0.0:9090 --require-auth
```

- `-v maharit-data:/data` を付けないと、コンテナを削除したときにデータも消えます
- コマンドを指定すると既定のコマンドは置き換わるので、`--host 0.0.0.0` と `--metrics-bind 0.0.0.0:9090` も書いてください。コンテナの中で `127.0.0.1` に bind すると、外から接続できません
- 認証や TLS などのオプションは [サーバー設定](server-config.md) を参照してください。パスワードはコマンドライン引数ではなく環境変数で渡すと、`docker inspect` やプロセス一覧に残りにくくなります

停止するときは `docker stop` を使ってください。SIGTERM を受け取ったサーバーはデータファイルを保存してから終了します。既定の猶予（10 秒）で保存が終わらない大きなデータでは、`docker stop -t 60 maharit` のように延ばしてください。

## REPL を使う

```bash
docker run -it --rm maharit-db /app/maharit
```

REPL のデータはメモリ上にだけあり、終了すると消えます。

## docker-compose

`docker-compose.yml` には次のサービスがあります。

| サービス | 内容 |
|---|---|
| `maharit-server` | サーバー（7687 / 9090 を公開、ボリューム `maharit-data` を `/data` に） |
| `maharit` | REPL（`docker-compose run maharit`） |
| `maharit-viz` | 可視化 Web アプリ（8080、`maharit-server` に接続） |

```bash
docker-compose up -d maharit-server
docker-compose run --rm maharit
```

リーダー 1 台・フォロワー 2 台のレプリケーション構成は `docker-compose.replication.yml` にあります（[レプリケーション](../advanced/replication.md)）。

## 環境変数

`maharit server` の環境変数はそのまま使えます（[サーバー設定](server-config.md)）。

| 変数 | イメージでの既定値 | 説明 |
|---|---|---|
| `MAHARIT_DATA` | `/data/maharit.db` | データファイルのパス |
| `MAHARIT_WAL` | （`fsync`） | WAL のモード |
| `MAHARIT_ADMIN_PASSWORD` | なし | `--require-auth` の admin パスワード |
| `MAHARIT_TLS_CERT` / `MAHARIT_TLS_KEY` | なし | TLS の証明書と秘密鍵（ボリュームでマウントしたファイルのパス） |
| `RUST_LOG` | （`info`） | [ログ](tracing.md)のレベル |

## バックアップ

サーバーのコンテナを止めてから、同じボリュームをマウントした別のコンテナで `maharit backup` を実行します（[バックアップ・リストア](backup.md)）。

```bash
docker stop maharit
docker run --rm -v maharit-data:/data -v "$PWD/backup:/backup" maharit-db \
  /app/maharit backup --source /data/maharit.db --output /backup/maharit.bak.gz --compress
docker start maharit
```

`/backup` にマウントするホストのディレクトリは、コンテナの `maharit` ユーザーが書き込めるようにしておいてください。

## 接続できないとき

1. コンテナが動いているか: `docker ps`
2. ポートの対応: `docker port maharit`
3. ログ: `docker logs maharit`（`Server listening on 0.0.0.0:7687` が出ているか）
4. `--host 0.0.0.0` を付けずにコマンドを上書きしていないか
