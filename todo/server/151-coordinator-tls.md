# タスク 151: コーディネーターとシャードの通信を TLS にする

## 現状（2026-11-08）
- `maharit server --coordinator` のクライアント待ち受け（`coordinator.rs`）は平文のみ。TLS を指定すると起動を拒否している（server/147）
- コーディネーターからシャードへの接続（`maharit-cluster` の `ShardClient`）は平文のみ。シャードを `--tls-cert` 付きで起動すると、コーディネーターから接続できない
- `ShardClient` は `sessionToken` を送らないため、シャードを `--require-auth` で起動しても接続できない

## 作業
- [ ] コーディネーターの待ち受けを `TcpServer` と同じく `ReloadableTlsAcceptor` で TLS にする
- [ ] `ShardClient` を TLS 対応にする（フラグ案: `--shard-ca <PATH>`）。maharit-cluster に rustls / tokio-rustls を追加
- [ ] シャードの認証（コーディネーター用の資格情報でログインして `sessionToken` を付ける）
- [ ] テストと docs（advanced/sharding.md、operations/tls.md の「TLS で保護されない通信」）
