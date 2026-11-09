---
title: TCP プロトコル仕様
parent: API リファレンス
nav_order: 3
---

# TCP プロトコル仕様

MaharitDB クライアントとサーバー間の通信プロトコルを説明します。独自のクライアントを実装する場合や、低レベルの通信を理解したい場合に参照してください。
定義の正本は `crates/maharit-server/src/tcp_server.rs` の `Request` / `Response` です。

## 概要

TCP ソケット（オプションで TLS）の上で、長さプレフィックス付きの JSON メッセージをやり取りします。ハンドシェイクはなく、接続直後からリクエストを送れます。

- クライアントがリクエストを 1 つ送り、サーバーがレスポンスを返す（ストリーミングクエリだけは複数のレスポンスが続く）
- 1 つの接続でリクエストを順番に何度でも送れる。応答は送った順に返る

## メッセージフォーマット

すべてのメッセージ（両方向）は次の形式です。

```text
+---------------------+---------------------+
| Length (4 bytes)    | Payload (N bytes)   |
+---------------------+---------------------+
  uint32 big-endian     UTF-8 JSON
```

- **Length**: ペイロードのバイト長（ビッグエンディアン 32 ビット符号なし整数）。上限は 64 MiB（`MAX_MESSAGE_SIZE`）で、超えるとエラーを返す
- **Payload**: JSON オブジェクト。`type` フィールドでメッセージの種類を表す

フィールド名は大文字小文字を区別します。省略可能なフィールドは省略してかまいません。

## リクエスト（Client→Server）

| `type` | 説明 | フィールド |
|--------|------|-----------|
| `query` | クエリを実行し、結果をまとめて返す | `query`、`params`?、`txId`?、`sessionToken`? |
| `streamQuery` | クエリを実行し、結果を分割して返す | `query`、`params`?、`chunkSize`?（既定 100）、`txId`?、`sessionToken`? |
| `login` | ログインしてセッショントークンを得る | `username`、`password` |
| `begin` | トランザクションを開始する | `readOnly`?（既定 `false`）、`sessionToken`? |
| `commit` | トランザクションをコミットする | `txId`、`sessionToken`? |
| `rollback` | トランザクションをロールバックする | `txId`、`sessionToken`? |
| `ping` | 死活確認 | なし |
| `stats` | サーバーの統計を取得する | なし |
| `promote` | このフォロワーをリーダーに昇格させる（admin のみ。[レプリケーション](../advanced/replication.md)） | `sessionToken`? |
| `disconnect` | 接続を終了する | なし |

`sessionToken` は、サーバーを `--require-auth` で起動したときに必要です（[認証](../operations/auth.md)）。

## レスポンス（Server→Client）

| `type` | 対応するリクエスト | フィールド |
|--------|-------------------|-----------|
| `result` | `query` | `rows` |
| `streamStart` / `streamChunk` / `streamEnd` | `streamQuery` | 下記 |
| `loggedIn` | `login` | `sessionToken`、`role`、`expiresAt` |
| `transactionBegun` | `begin` | `txId` |
| `committed` | `commit` | `txId` |
| `rolledBack` | `rollback` | `txId` |
| `pong` | `ping` | なし |
| `stats` | `stats` | `connections`、`total_queries`、`nodes`、`edges`、`replication`? |
| `promoted` | `promote` | `replicationBind`?（フォロワーの接続を受け付け始めたアドレス） |
| `goodbye` | `disconnect`、またはサーバー停止時 | なし |
| `error` | どれでも | `message` |
| `authError` | どれでも | `message`（トークンがない・無効・期限切れ、権限不足、ログイン失敗） |

## クエリ

### query

```json
{
  "type": "query",
  "query": "MATCH (n:Person {name: $name}) WHERE n.age >= $min_age RETURN n.name AS name, n.age AS age",
  "params": {"name": "Alice", "min_age": 25}
}
```

```json
{
  "type": "result",
  "rows": [
    {"name": "Alice", "age": 30}
  ]
}
```

- `rows` は行の配列で、各行は「列名 → 値」のオブジェクト
- 値は JSON の型を保つ（文字列・数値・真偽値・`null`・配列・オブジェクト）。ノード、パス、日付・日時・期間は文字列で返る
- 書き込みクエリは件数を返す（例: `{"created_nodes": 1, "created_edges": 0}`、`{"properties_set": 1}`）

### クエリパラメータ（`params`）

`params` は、クエリ中の `$name` に渡す値のオブジェクトです。値はクエリ文字列とは別に送られ、Cypher として解釈されることはありません。**利用者の入力は、クエリ文字列に埋め込まずに `params` で渡してください。**

| JSON | Cypher の型 |
|------|------------|
| `null` | null |
| `true` / `false` | Boolean |
| 整数（i64 に収まる数） | Integer |
| それ以外の数 | Float |
| 文字列 | String |
| 配列 | List |
| オブジェクト | Map |

クエリが参照しているパラメータが `params` にないと `error` になります（`null` 扱いにはなりません）。`params` を省略した場合や空オブジェクトの場合は、パラメータなしで実行します。
使い方の詳細は [パラメータ（$param 構文）](../cypher/parameters.md) を参照してください。

### streamQuery

```json
{"type": "streamQuery", "query": "MATCH (n:Person) RETURN n.name AS name", "chunkSize": 2}
```

サーバーは次の順にレスポンスを送ります。

```json
{"type": "streamStart", "streamId": 0, "totalRows": 3}
{"type": "streamChunk", "streamId": 0, "chunkIndex": 0, "rows": [{"name": "Alice"}, {"name": "Bob"}]}
{"type": "streamChunk", "streamId": 0, "chunkIndex": 1, "rows": [{"name": "Charlie"}]}
{"type": "streamEnd", "streamId": 0, "totalRows": 3}
```

クエリが失敗した場合は、`streamStart` の代わりに `error` が 1 つ返ります。

## 認証

```json
{"type": "login", "username": "alice", "password": "secret"}
```

```json
{"type": "loggedIn", "sessionToken": "3f9c…", "role": "read_write", "expiresAt": 1767225600}
```

- `role` は `admin` / `read_write` / `read_only`
- `expiresAt` はセッションの有効期限（Unix エポック秒）
- 以降のリクエストに `"sessionToken": "3f9c…"` を付ける

## トランザクション

```json
{"type": "begin"}
{"type": "query", "query": "CREATE (:Item {id: $id})", "params": {"id": 1}, "txId": 7}
{"type": "commit", "txId": 7}
```

`begin` への応答 `{"type": "transactionBegun", "txId": 7}` の `txId` を、`query` と `commit` / `rollback` に付けます。

## エラー

```json
{"type": "error", "message": "Parse error: …"}
```

`message` は人が読むための文字列で、次のような接頭辞が付きます。

| 接頭辞 | 意味 |
|--------|------|
| `Invalid request:` | JSON として読めない、`type` が不明、必須フィールドがない |
| `Parse error:` | クエリの構文エラー |
| `Execution error:` | 実行時エラー（制約違反、型エラー、未定義のパラメータなど） |

エラーを返したあとも接続はそのまま使えます。

## 接続の終了

クライアントは `{"type": "disconnect"}` を送ります。サーバーは `{"type": "goodbye"}` を返して接続を閉じます。サーバーの停止時にも、サーバーから `goodbye` が送られます。

## 実装例

Python（標準ライブラリのみ）:

```python
import json
import socket
import struct

def send(sock, msg):
    body = json.dumps(msg).encode("utf-8")
    sock.sendall(struct.pack(">I", len(body)) + body)

def recv(sock):
    def read_exact(n):
        buf = b""
        while len(buf) < n:
            chunk = sock.recv(n - len(buf))
            if not chunk:
                raise ConnectionError("connection closed")
            buf += chunk
        return buf
    (length,) = struct.unpack(">I", read_exact(4))
    return json.loads(read_exact(length))

with socket.create_connection(("127.0.0.1", 7687)) as sock:
    send(sock, {
        "type": "query",
        "query": "MATCH (n:Person {name: $name}) RETURN n.age AS age",
        "params": {"name": "Alice"},
    })
    print(recv(sock))  # {'type': 'result', 'rows': [{'age': 30}]}
```
