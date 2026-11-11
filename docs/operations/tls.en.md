---
title: TLS/SSL
parent: Server and operations
nav_order: 2
source_commit: 6e4c993e
---

# TLS/SSL

Give `maharit server` a certificate and private key to encrypt client connections (port 7687 by default) with TLS 1.2/1.3 (using rustls). Without them, the server listens in plaintext.

On a replication leader, the replication connection (`--replication-bind`, port 7688 by default) uses TLS with the same certificate. Followers verify the leader's certificate with `--leader-ca` (see [Replication](../advanced/replication.md)).

On a coordinator (`--coordinator`), `--tls-cert` / `--tls-key` enable TLS for client connections and `--shard-ca` enables TLS for connections to shards (see [Sharding](../advanced/sharding.md)).

!!! warning "Traffic not protected by TLS"
    The metrics and health-check HTTP server (`--metrics-bind`, `127.0.0.1:9090` by default, loopback only) is plaintext. If you expose it beyond loopback, keep it inside a trusted network.

## Server settings

```bash
maharit server \
  --host 0.0.0.0 \
  --tls-cert /etc/maharit/server.crt \
  --tls-key /etc/maharit/server.key
```

| Option | Environment variable | Description |
|-----------|---------|------|
| `--tls-cert <PATH>` | `MAHARIT_TLS_CERT` | Server certificate in PEM. Put any intermediate certificates in the same file after the server certificate |
| `--tls-key <PATH>` | `MAHARIT_TLS_KEY` | Private key in PEM (PKCS#8, PKCS#1 (RSA), or SEC1 (EC)) |
| `--tls-min-version <1.2\|1.3>` | `MAHARIT_TLS_MIN_VERSION` | Minimum TLS version accepted (default `1.2`) |

At startup it prints:

```text
TLS enabled (certificate: /etc/maharit/server.crt)
Server listening on 0.0.0.0:7687 (TLS)
```

- Give both the certificate and the key. If only one is given, a file cannot be read, or the certificate and key do not match, startup aborts instead of falling back to plaintext.
- Listening on a non-loopback address (such as `--host 0.0.0.0`) without TLS logs a warning that traffic is unencrypted.
- Connections that do not finish the TLS handshake within `read_timeout` (30 seconds) are closed. Clients that connect in plaintext get no reply.

## Preparing certificates

### Certificates from a public CA (Let's Encrypt and others)

```bash
sudo certbot certonly --standalone -d db.example.com
```

Pass `fullchain.pem` (including intermediate certificates) to `--tls-cert` and `privkey.pem` to `--tls-key`.

```bash
maharit server --host 0.0.0.0 \
  --tls-cert /etc/letsencrypt/live/db.example.com/fullchain.pem \
  --tls-key /etc/letsencrypt/live/db.example.com/privkey.pem
```

### Self-signed certificates (development and internal use)

On the client side, give this certificate file itself as the trusted CA.

```bash
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
  -keyout server.key -out server.crt -days 365 \
  -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" \
  -addext "basicConstraints=critical,CA:FALSE"
```

- Clients check that the host name (or IP address) they connect to is in `subjectAltName`. Put the host name you actually connect to there.
- Do not omit `basicConstraints=critical,CA:FALSE`. By default OpenSSL creates a CA certificate (`CA:TRUE`), which the Rust client (rustls) rejects with `CaUsedAsEndEntity`.

## Client settings

### Python

Give the trusted CA certificate (the server certificate itself when self-signed) as `ca_file` to connect over TLS. For a certificate from a public CA, `ssl=True` is enough.

```python
from maharit import AsyncClient, Client, ConnectionPool

# Self-signed certificate
with Client.connect("localhost:7687", ca_file="server.crt") as client:
    client.query("MATCH (n) RETURN count(n)")

# Certificate from a public CA (verified with the system CA store)
with Client.connect("db.example.com:7687", ssl=True) as client:
    ...

# The async client and the connection pool take the same arguments
client = await AsyncClient.connect("localhost:7687", ca_file="server.crt")
pool = ConnectionPool("localhost:7687", ca_file="server.crt")
```

`ssl` also accepts an `ssl.SSLContext` (for finer control, such as client certificates).

### Rust (`maharit-client`)

```rust
use maharit_client::{Client, TlsClientConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The first argument is the host name to verify the certificate against
    let tls = TlsClientConfig::new("localhost").with_ca_cert("server.crt");
    let mut client = Client::connect_tls("localhost:7687", tls).await?;
    client.ping().await?;
    Ok(())
}
```

Without `with_ca_cert`, the certificate is verified against public CAs (Mozilla's root certificates). `with_skip_verify(true)` disables verification; use it only in tests.

## Renewing certificates (hot reload)

The server checks the modification times of the certificate and key files every 30 seconds and reloads them when they change. No restart is needed.

- New connections after the reload use the new certificate. Established connections continue as they are.
- If the new files cannot be read, or the certificate and key do not match, the error is logged and the previous certificate stays in use.
- Write the key and certificate to temporary files and swap them in with `mv` so a half-written file is never read. certbot renewals work as is.

After a reload, the server logs:

```text
TLS certificate reloaded from /etc/maharit/server.crt
```

## Troubleshooting

| Symptom | Cause and fix |
|------|-----------|
| `certificate verify failed` / `UnknownIssuer` on the client | The client does not trust the certificate. For a self-signed one, give the server certificate as `ca_file` / `with_ca_cert` |
| `CaUsedAsEndEntity` | The self-signed certificate was created with `CA:TRUE`. Recreate it with `-addext "basicConstraints=critical,CA:FALSE"` |
| `NotValidForName` / `Hostname mismatch` | The host name you connect to is not in the certificate's `subjectAltName` |
| `TLS handshake failed` in the server log | The client connected in plaintext or rejected the certificate |
| `failed to load the TLS certificate/key` at startup | Wrong path, not PEM, or the certificate and key are swapped |

Check a certificate's validity period with:

```bash
openssl x509 -in server.crt -noout -dates
```
