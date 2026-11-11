---
title: ログ
parent: サーバー・運用
nav_order: 6
---

# ログ

`maharit server` は、動作の記録を JSON 形式の構造化ログとして**標準出力**に 1 行ずつ書き出します。起動メッセージなど一部の人向けの表示も同じ標準出力に出ます。

!!! note "OpenTelemetry"
    OpenTelemetry（OTLP）へのトレースの送信には、まだ対応していません。クエリごとの処理時間は、下記のログの `query completed` で確認できます。

## 出力例

```text
{"timestamp":"2026-11-11T22:53:30.745852Z","level":"INFO","fields":{"message":"Server listening","address":"127.0.0.1:7687","transport":"plaintext"},"target":"maharit::tcp_server","filename":"crates/maharit-server/src/tcp_server.rs","line_number":1087}
{"timestamp":"2026-11-11T22:53:31.915345Z","level":"INFO","fields":{"message":"Client connected","peer":"127.0.0.1:54300"},"target":"maharit::tcp_server","filename":"crates/maharit-server/src/tcp_server.rs","line_number":1167}
{"timestamp":"2026-11-11T22:53:31.915482Z","level":"INFO","fields":{"message":"query completed","duration_us":18},"target":"maharit::tcp_server","filename":"crates/maharit-server/src/tcp_server.rs","line_number":1441,"span":{"query":"CREATE (:A)","name":"query"},"spans":[{"query":"CREATE (:A)","name":"query"}]}
{"timestamp":"2026-11-11T22:53:31.915741Z","level":"INFO","fields":{"message":"Client disconnected","peer":"127.0.0.1:54300"},"target":"maharit::tcp_server","filename":"crates/maharit-server/src/tcp_server.rs","line_number":1230}
```

| フィールド | 内容 |
|---|---|
| `timestamp` | UTC の時刻 |
| `level` | `ERROR` / `WARN` / `INFO` / `DEBUG` / `TRACE` |
| `fields.message` | イベントの内容 |
| `fields.*` | イベントごとの値（`peer`、`duration_us` など） |
| `span` / `spans` | 実行中の処理。クエリのログでは `span.query` にクエリ文が入る |
| `target` / `filename` / `line_number` | 出力したモジュールとソースの位置 |

## 主なイベント

| `message` | レベル | 値 |
|---|---|---|
| `Server listening` | INFO | `address`、`transport`（`plaintext` / `tls`） |
| `Client connected` / `Client disconnected` | INFO | `peer` |
| `Connection error` | WARN | `peer`、`error` |
| `query completed` | INFO | `duration_us`（マイクロ秒）、`span.query` |
| `streaming query` | INFO | `query` |
| `checkpoint completed` | INFO | |
| `promoted to leader` | WARN | `replication_bind` |

認証が無効のとき、平文でネットワークに公開しているときは、起動時に WARN を出します。

!!! warning "クエリ文がログに残る"
    `query completed` にはクエリ文がそのまま入ります。クエリに個人情報などを直接書くとログにも残るため、値はパラメータ（`$name`）で渡してください（パラメータの値はログに出ません）。`CREATE USER` / `ALTER USER` の文はログに出ません。

## 出力するレベルを変える

環境変数 `RUST_LOG` で出力するレベルを指定します（既定: `info`）。

```bash
# 警告とエラーだけ
RUST_LOG=warn maharit server

# クエリのログを止める（tcp_server の INFO を出さない）
RUST_LOG=info,maharit::tcp_server=warn maharit server
```

## ログを集める

標準出力を、systemd の journal や Docker のログドライバー、Fluent Bit / Vector などに渡します。1 行 1 JSON なので、そのまま取り込めます。

```bash
maharit server --data /var/lib/maharit/maharit.db >> /var/log/maharit/server.log
```

JSON でない行（`Server listening on …` などの人向けの表示）も混ざるので、取り込み側で JSON として読めない行を捨てるか、そのまま文字列として扱ってください。
