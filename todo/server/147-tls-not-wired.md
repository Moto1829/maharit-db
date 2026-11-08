# タスク 147: TLS が TCP サーバーに組み込まれていない

## 現状（2026-11-08 確認）
- `crates/maharit-server/src/tls.rs` に rustls の設定・証明書ホットリロード・`MAHARIT_TLS_CERT` / `MAHARIT_TLS_KEY` からの読み込み（`from_env`）があるが、`main.rs` と `tcp_server.rs` のどこからも使われていない。`maharit server` は常に平文で待ち受ける
- `--tls-cert` / `--tls-key` フラグもない（旧 `server-config.md` には記載があった）
- README・docs/index.md・architecture は「TLS 1.2/1.3 対応の TCP サーバー」と書いている。`docs/operations/server-config.md` には未組み込みの注意を書いた（2026-11-08）
- Rust クライアント（`maharit-client` の `TlsClientConfig`）側は TLS 接続できる作りになっている

## 作業
- [ ] `--tls-cert` / `--tls-key`（と環境変数）を追加し、`TcpServer` の accept 後に `TlsAcceptor` で包む（`TcpStream` 前提の `handle_connection` / `read_message` / `send_response` を `AsyncRead + AsyncWrite` に一般化）
- [ ] 証明書のホットリロードを配線
- [ ] レプリケーション接続・メトリクス HTTP の TLS をどうするか決める
- [ ] Rust / Python クライアントで TLS 接続する E2E
- [ ] docs（operations/tls.md・server-config.md）・README の記述を実装に合わせる
