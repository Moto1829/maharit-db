---
title: TLS/SSL 設定
parent: サーバー・運用
nav_order: 2
---

# TLS/SSL 設定

`maharit server` に証明書と秘密鍵を渡すと、クライアント接続（デフォルト 7687 番ポート）を TLS 1.2/1.3 で暗号化します（rustls を使用）。指定しない場合は平文で待ち受けます。

レプリケーションのリーダーでは、レプリケーション接続（`--replication-bind`、デフォルト 7688 番）にも同じ証明書で TLS をかけます。フォロワーは `--leader-ca` でリーダーの証明書を検証して接続します（[レプリケーション](../advanced/replication.md)）。

!!! warning "TLS で保護されない通信"
    次の通信は平文のままなので、信頼できるネットワーク内に閉じてください。

    - メトリクス・ヘルスチェックの HTTP（`--metrics-bind`、デフォルトは `127.0.0.1:9090` でループバックのみ）
    - コーディネーターとシャードの間の通信。コーディネーターモードでは TLS を指定すると起動を拒否します

## サーバーの設定

```bash
maharit server \
  --host 0.0.0.0 \
  --tls-cert /etc/maharit/server.crt \
  --tls-key /etc/maharit/server.key
```

| オプション | 環境変数 | 説明 |
|-----------|---------|------|
| `--tls-cert <PATH>` | `MAHARIT_TLS_CERT` | PEM 形式のサーバー証明書。中間証明書がある場合は、サーバー証明書に続けて同じファイルに入れる |
| `--tls-key <PATH>` | `MAHARIT_TLS_KEY` | PEM 形式の秘密鍵（PKCS#8、PKCS#1（RSA）、SEC1（EC）） |
| `--tls-min-version <1.2\|1.3>` | `MAHARIT_TLS_MIN_VERSION` | 受け付ける最小の TLS バージョン（デフォルト `1.2`） |

起動すると、次のように表示されます。

```text
TLS enabled (certificate: /etc/maharit/server.crt)
Server listening on 0.0.0.0:7687 (TLS)
```

- 証明書と秘密鍵は両方指定します。片方だけの場合や、ファイルが読めない、証明書と鍵が対応しない場合は、平文に戻さずに起動を中止します。
- TLS を指定せずにループバック以外のアドレス（`--host 0.0.0.0` など）で待ち受けると、通信が暗号化されていないことを警告します。
- TLS ハンドシェイクが `read_timeout`（30 秒）以内に終わらない接続は切断します。平文で接続してきたクライアントには応答しません。

## 証明書の準備

### 公的な CA の証明書（Let's Encrypt など）

```bash
sudo certbot certonly --standalone -d db.example.com
```

`fullchain.pem`（中間証明書を含む）を `--tls-cert` に、`privkey.pem` を `--tls-key` に指定します。

```bash
maharit server --host 0.0.0.0 \
  --tls-cert /etc/letsencrypt/live/db.example.com/fullchain.pem \
  --tls-key /etc/letsencrypt/live/db.example.com/privkey.pem
```

### 自己署名証明書（開発・社内向け）

クライアント側では、この証明書ファイルそのものを「信頼する CA」として指定します。

```bash
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
  -keyout server.key -out server.crt -days 365 \
  -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" \
  -addext "basicConstraints=critical,CA:FALSE"
```

- クライアントは接続先のホスト名（または IP アドレス）が `subjectAltName` に含まれているかを検証します。実際に接続するホスト名を入れてください。
- `basicConstraints=critical,CA:FALSE` は省略しないでください。OpenSSL の既定では CA 証明書（`CA:TRUE`）が作られ、Rust クライアント（rustls）が `CaUsedAsEndEntity` で接続を拒否します。

## クライアントの設定

### Python

`ca_file` に信頼する CA 証明書（自己署名の場合はサーバー証明書）を指定すると、TLS で接続します。公的な CA の証明書なら `ssl=True` だけで接続できます。

```python
from maharit import AsyncClient, Client, ConnectionPool

# 自己署名証明書
with Client.connect("localhost:7687", ca_file="server.crt") as client:
    client.query("MATCH (n) RETURN count(n)")

# 公的な CA の証明書（システムの CA ストアで検証）
with Client.connect("db.example.com:7687", ssl=True) as client:
    ...

# 非同期クライアントとコネクションプールも同じ引数
client = await AsyncClient.connect("localhost:7687", ca_file="server.crt")
pool = ConnectionPool("localhost:7687", ca_file="server.crt")
```

`ssl` には `ssl.SSLContext` も渡せます（クライアント証明書の設定など、細かく制御したい場合）。

### Rust（`maharit-client`）

```rust
use maharit_client::{Client, TlsClientConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 第 1 引数は証明書を検証するホスト名
    let tls = TlsClientConfig::new("localhost").with_ca_cert("server.crt");
    let mut client = Client::connect_tls("localhost:7687", tls).await?;
    client.ping().await?;
    Ok(())
}
```

`with_ca_cert` を省略すると、公的な CA（Mozilla のルート証明書）で検証します。`with_skip_verify(true)` は検証を無効にするので、テスト以外では使わないでください。

## 証明書の更新（ホットリロード）

サーバーは証明書と秘密鍵のファイルの更新時刻を 30 秒ごとに確認し、変わっていれば読み込み直します。再起動は不要です。

- 読み込み直した後の新しい接続から、新しい証明書を使います。確立済みの接続はそのまま続きます。
- 新しいファイルが読めない、または証明書と鍵が対応しない場合は、エラーを記録して前の証明書を使い続けます。
- 鍵と証明書は、それぞれ一時ファイルに書いてから `mv` で置き換えると、書きかけのファイルを読む心配がありません。certbot の更新はそのまま使えます。

読み込み直すと、サーバーのログに次のように出ます。

```text
TLS certificate reloaded from /etc/maharit/server.crt
```

## トラブルシューティング

| 症状 | 原因と対処 |
|------|-----------|
| クライアントで `certificate verify failed` / `UnknownIssuer` | クライアントが証明書を信頼していない。自己署名なら `ca_file` / `with_ca_cert` にサーバー証明書を指定する |
| `CaUsedAsEndEntity` | 自己署名証明書が `CA:TRUE` で作られている。`-addext "basicConstraints=critical,CA:FALSE"` を付けて作り直す |
| `NotValidForName` / `Hostname mismatch` | 接続先のホスト名が証明書の `subjectAltName` にない |
| サーバーログに `TLS handshake failed` | クライアントが平文で接続している、または証明書を拒否した |
| 起動時に `failed to load the TLS certificate/key` | パスの誤り、PEM でない、証明書と鍵の取り違え |

証明書の有効期限は次のコマンドで確認できます。

```bash
openssl x509 -in server.crt -noout -dates
```
