# タスク 147: TLS が TCP サーバーに組み込まれていない

## 現状（2026-11-08 確認）
- `crates/maharit-server/src/tls.rs` に rustls の設定・証明書ホットリロード・`MAHARIT_TLS_CERT` / `MAHARIT_TLS_KEY` からの読み込み（`from_env`）があるが、`main.rs` と `tcp_server.rs` のどこからも使われていない。`maharit server` は常に平文で待ち受ける
- `--tls-cert` / `--tls-key` フラグもない（旧 `server-config.md` には記載があった）
- README・docs/index.md・architecture は「TLS 1.2/1.3 対応の TCP サーバー」と書いている。`docs/operations/server-config.md` には未組み込みの注意を書いた（2026-11-08）
- Rust クライアント（`maharit-client` の `TlsClientConfig`）側は TLS 接続できる作りになっている

## 作業
- [x] `--tls-cert` / `--tls-key` / `--tls-min-version`（環境変数 `MAHARIT_TLS_*`）を追加し、accept 後の接続タスク内で TLS ハンドシェイク（`read_timeout` で打ち切り）。接続処理を `AsyncRead + AsyncWrite` に一般化
- [x] 証明書のホットリロード（`ReloadableTlsAcceptor`、30 秒ごとに更新時刻を確認。読み込み失敗時は前の証明書を維持）
- [x] レプリケーション接続・メトリクス HTTP・コーディネーター↔シャードは今回は平文のまま（docs に明記、コーディネーターで TLS 指定は起動拒否）。レプリケーションの TLS は server/150 に起票
- [x] E2E: `scripts/tls_test.py`（Python 同期/非同期、平文・未信頼証明書の拒否、設定誤りで起動拒否）を `run_e2e_local.sh` に追加。Rust クライアントはユニットテスト `tls_server_works_with_rust_client`
- [x] docs（operations/tls.md・server-config.md・api/rust-client.md・api/python-client.md）を実装に合わせる

## 実施メモ（2026-11-08）
- Python クライアント（`Client` / `AsyncClient` / 両プール）に `ssl=` / `ca_file=` を追加
- Rust クライアント: `with_ca_cert` 省略時にルート証明書が空で常に検証失敗していたのを webpki-roots で検証するよう修正。不要な ALPN（`http/1.1`）を削除
- TLS なしでループバック以外に bind すると起動時に警告
- 自己署名証明書は `CA:FALSE` で作らないと rustls が `CaUsedAsEndEntity` で拒否する（docs に手順を記載）
