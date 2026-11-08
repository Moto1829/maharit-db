# タスク 150: レプリケーション・コーディネーター間の通信を TLS にする

## 現状（2026-11-08、server/147 の後）
- クライアント接続（7687）は `--tls-cert` / `--tls-key` で TLS にできるようになった
- レプリケーション（`--replication-bind`、7688）は平文。`--replication-secret` のシークレットも `auth_token` として平文で送られるため、経路上で盗聴されるとフォロワーになりすませる
- コーディネーター（`--coordinator`）とシャードの間（`maharit-cluster` の `shard_client`）も平文。シャードを TLS で起動するとコーディネーターから接続できないため、コーディネーターモードで TLS を指定すると起動を拒否している
- メトリクス HTTP（`--metrics-bind`）も平文（既定はループバックのみ）

## 作業
- [ ] リーダーのレプリケーション待ち受けを TLS にし、フォロワーは CA 証明書で検証して接続する（フラグ案: `--replication-tls-cert` / `--replication-tls-key` / `--replication-ca`、またはクライアント用の証明書を流用）
- [ ] シークレットを平文で送らない方式（TLS 前提にする、またはチャレンジ・レスポンス）を検討
- [ ] `shard_client` を TLS 対応にし、コーディネーターモードの TLS 拒否を外す
- [ ] E2E（replication_test / failover_test を TLS 構成でも回す）と docs（operations/tls.md の「TLS で保護されない通信」）
