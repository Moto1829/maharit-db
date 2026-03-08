---
title: TLS/SSL 設定
parent: サーバー・運用
nav_order: 2
---

# TLS/SSL 設定

MaharitDB は rustls を使用した TLS 1.2/1.3 対応の暗号化通信をサポートしています。

## 証明書の準備

### 自己署名証明書の生成（開発用）

```bash
# 秘密鍵の生成
openssl genrsa -out server.key 2048

# 証明書署名要求（CSR）の生成
openssl req -new -key server.key -out server.csr \
  -subj "/C=JP/ST=Tokyo/L=Tokyo/O=MyOrg/CN=localhost"

# 自己署名証明書の生成（有効期間 365 日）
openssl x509 -req -days 365 -in server.csr -signkey server.key -out server.crt

# 確認
openssl x509 -in server.crt -text -noout
```

### Let's Encrypt を使用する場合

```bash
# certbot のインストール
sudo apt install certbot

# 証明書の取得
sudo certbot certonly --standalone -d yourdomain.com

# 証明書は以下のパスに保存される
# /etc/letsencrypt/live/yourdomain.com/fullchain.pem
# /etc/letsencrypt/live/yourdomain.com/privkey.pem
```

## サーバーの TLS 設定

証明書ファイルと秘密鍵ファイルをコマンドラインオプションで指定します。

```bash
maharit server \
  --host 0.0.0.0 \
  --port 7687 \
  --tls-cert /path/to/server.crt \
  --tls-key /path/to/server.key
```

環境変数での設定：

```bash
export MAHARIT_TLS_CERT=/etc/maharit/server.crt
export MAHARIT_TLS_KEY=/etc/maharit/server.key
maharit server --host 0.0.0.0
```

## クライアントの TLS 設定

### 自己署名証明書を使用する場合

サーバーの証明書をクライアント側で信頼するよう設定します。

```rust
use maharit_client::ClientBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = ClientBuilder::new("localhost:7687")
        .with_tls(true)
        .with_ca_cert("/path/to/server.crt")  // 自己署名証明書の場合
        .build()
        .await?;

    Ok(())
}
```

### 正式な CA 証明書を使用する場合

```rust
use maharit_client::ClientBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = ClientBuilder::new("yourdomain.com:7687")
        .with_tls(true)
        // CA 証明書が信頼されている場合は追加設定不要
        .build()
        .await?;

    Ok(())
}
```

## TLS バージョン

MaharitDB は TLS 1.2 および TLS 1.3 をサポートします。セキュリティ上の理由から TLS 1.0 および 1.1 は使用できません。

```
対応 TLS バージョン:
- TLS 1.2 (最低要件)
- TLS 1.3 (推奨)
```

## 対応している暗号スイート（TLS 1.3）

- `TLS_AES_256_GCM_SHA384`
- `TLS_AES_128_GCM_SHA256`
- `TLS_CHACHA20_POLY1305_SHA256`

## 証明書のホットリロード

サーバーを停止せずに証明書を更新できます。

```bash
# 新しい証明書を配置した後、SIGHUP シグナルを送信
kill -HUP $(pgrep maharit)
```

サーバーログで確認：

```json
{"level":"INFO","message":"Reloading TLS certificates","timestamp":"2024-06-01T00:00:00Z"}
{"level":"INFO","message":"TLS certificates reloaded successfully","timestamp":"2024-06-01T00:00:00Z"}
```

## mTLS（クライアント証明書認証）

クライアント証明書による相互 TLS 認証にも対応しています。

```bash
# クライアント証明書付きでサーバーを起動
maharit server \
  --tls-cert /etc/maharit/server.crt \
  --tls-key /etc/maharit/server.key \
  --tls-ca /etc/maharit/ca.crt  # クライアント証明書の検証に使用する CA
```

```rust
// クライアント証明書を使用して接続
let client = ClientBuilder::new("localhost:7687")
    .with_tls(true)
    .with_client_cert("/path/to/client.crt", "/path/to/client.key")
    .build()
    .await?;
```

## トラブルシューティング

### "certificate verify failed" エラー

自己署名証明書を使用している場合、クライアント側で CA 証明書を指定してください。

### "TLS handshake timeout" エラー

ファイアウォールがポートをブロックしていないか確認してください。

### 証明書の有効期限確認

```bash
openssl x509 -in server.crt -noout -dates
```
