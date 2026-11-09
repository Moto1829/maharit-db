# タスク 151: コーディネーターとシャードの通信を TLS にする

## 現状（2026-11-08）
- `maharit server --coordinator` のクライアント待ち受け（`coordinator.rs`）は平文のみ。TLS を指定すると起動を拒否している（server/147）
- コーディネーターからシャードへの接続（`maharit-cluster` の `ShardClient`）は平文のみ。シャードを `--tls-cert` 付きで起動すると、コーディネーターから接続できない
- `ShardClient` は `sessionToken` を送らないため、シャードを `--require-auth` で起動しても接続できない

## 作業
- [x] コーディネーターの待ち受けを `ReloadableTlsAcceptor` で TLS に（`--tls-cert` / `--tls-key`、証明書のホットリロード、平文で外部公開時の警告）
- [x] `ShardClient` を TLS 対応に（`--shard-ca`、`ShardConnectOptions`）。maharit-cluster に rustls / tokio-rustls を追加
- [x] シャードの認証（`--shard-user` / `--shard-password`、ログインして `sessionToken` を付ける。セッション切れで 1 回ログインし直す）
- [x] テスト: 単体（TLS + 認証のシャード、パスワード誤り、未信頼の証明書、平文、セッション切れ後の再ログイン、TLS のコーディネーター）。E2E: tls_test.py にコーディネーター構成
- [x] docs（advanced/sharding.md にコーディネーターサーバーの節と制限、operations/tls.md、server-config.md）

## メモ（2026-11-09）
- 作業前に、コーディネーターが数値を返せず、エラーを空の結果にしていた不具合を bug/153 として別コミットで修正
- コーディネーターのクエリの意味（全シャードへの書き込み、集計、重複除去）は server/154
