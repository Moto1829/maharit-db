---
title: TCP プロトコル仕様
parent: API リファレンス
nav_order: 3
---

# TCP プロトコル仕様

MaharitDB クライアントとサーバー間の通信プロトコルを説明します。このドキュメントは独自クライアントを実装する場合や、低レベルの通信を理解したい場合に参照してください。

## 概要

MaharitDB は TCP ソケット上で独自のバイナリプロトコルを使用します。オプションで TLS による暗号化に対応しています。

```
[TCP Connection]
  ↓
[TLS Layer（オプション）]
  ↓
[MaharitDB Protocol]
  ├── ハンドシェイク
  ├── 認証
  └── クエリ/応答
```

## メッセージフォーマット

すべてのメッセージは以下の形式です：

```
+------------------+------------------+------------------+
| Message Type (1) | Payload Len (4)  | Payload (N)      |
+------------------+------------------+------------------+
  uint8              uint32 big-endian  JSON or binary
```

- **Message Type**: メッセージの種類を示す 1 バイトの識別子
- **Payload Length**: ペイロードのバイト長（ビッグエンディアン 32 ビット整数）
- **Payload**: メッセージの本体（JSON エンコードまたはバイナリ）

## メッセージタイプ

| 値 | 名前 | 方向 | 説明 |
|----|------|------|------|
| `0x01` | `Hello` | Client→Server | ハンドシェイク開始 |
| `0x02` | `Welcome` | Server→Client | ハンドシェイク応答 |
| `0x10` | `Auth` | Client→Server | 認証情報送信 |
| `0x11` | `AuthOk` | Server→Client | 認証成功 |
| `0x12` | `AuthError` | Server→Client | 認証失敗 |
| `0x20` | `Query` | Client→Server | クエリ送信 |
| `0x21` | `QueryResult` | Server→Client | クエリ結果 |
| `0x22` | `QueryError` | Server→Client | クエリエラー |
| `0x30` | `Goodbye` | 両方 | 接続終了 |
| `0xFF` | `Error` | Server→Client | プロトコルエラー |

## ハンドシェイク

### Hello メッセージ（Client→Server）

```json
{
  "protocol_version": 1,
  "client_name": "maharit-client-rust",
  "client_version": "0.1.0"
}
```

### Welcome メッセージ（Server→Client）

```json
{
  "protocol_version": 1,
  "server_version": "0.1.0",
  "server_id": "maharit-prod-01",
  "auth_required": true
}
```

## 認証

### Auth メッセージ（Client→Server）

```json
{
  "username": "alice",
  "password": "password_hash_or_token"
}
```

### AuthOk メッセージ（Server→Client）

```json
{
  "session_token": "abc123...",
  "expires_at": "2024-01-02T00:00:00Z",
  "role": "reader"
}
```

## クエリの送受信

### Query メッセージ（Client→Server）

```json
{
  "query_id": "q-12345",
  "statement": "MATCH (n:Person {name: $name}) RETURN n",
  "params": {
    "name": "Alice"
  }
}
```

- `query_id`: クライアントが生成する一意な識別子（応答の対応付けに使用）

### QueryResult メッセージ（Server→Client）

```json
{
  "query_id": "q-12345",
  "columns": ["n"],
  "rows": [
    [{"id": 1, "labels": ["Person"], "properties": {"name": "Alice", "age": 30}}]
  ],
  "stats": {
    "nodes_created": 0,
    "nodes_deleted": 0,
    "edges_created": 0,
    "properties_set": 0,
    "execution_time_ms": 2
  }
}
```

### QueryError メッセージ（Server→Client）

```json
{
  "query_id": "q-12345",
  "error_code": "PARSE_ERROR",
  "message": "Unexpected token 'FORM' at line 1, column 7",
  "details": {
    "line": 1,
    "column": 7,
    "token": "FORM"
  }
}
```

## エラーコード

| コード | 説明 |
|--------|------|
| `PARSE_ERROR` | クエリの構文エラー |
| `EXECUTION_ERROR` | クエリ実行エラー |
| `CONSTRAINT_VIOLATION` | 制約違反 |
| `AUTH_REQUIRED` | 認証が必要 |
| `PERMISSION_DENIED` | 権限なし |
| `TIMEOUT` | クエリタイムアウト |
| `INTERNAL_ERROR` | サーバー内部エラー |

## 接続の終了

```json
// Goodbye メッセージ（どちらからでも送信可能）
{
  "reason": "client_disconnect"
}
```

## 実装例

```rust
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn send_message(
    stream: &mut TcpStream,
    msg_type: u8,
    payload: &[u8],
) -> Result<(), std::io::Error> {
    // メッセージタイプ
    stream.write_u8(msg_type).await?;
    // ペイロード長（ビッグエンディアン）
    stream.write_u32(payload.len() as u32).await?;
    // ペイロード
    stream.write_all(payload).await?;
    Ok(())
}

async fn recv_message(
    stream: &mut TcpStream,
) -> Result<(u8, Vec<u8>), std::io::Error> {
    let msg_type = stream.read_u8().await?;
    let len = stream.read_u32().await? as usize;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await?;
    Ok((msg_type, payload))
}
```
