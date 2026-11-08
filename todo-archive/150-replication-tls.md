# タスク 150: レプリケーションの通信を TLS にする

## 現状（2026-11-08、server/147 の後）
- クライアント接続（7687）は `--tls-cert` / `--tls-key` で TLS にできるようになった
- レプリケーション（`--replication-bind`、7688）は平文。`--replication-secret` のシークレットも `auth_token` として平文で送られるため、経路上で盗聴されるとフォロワーになりすませる

## 作業
- [x] リーダーで `--tls-cert` / `--tls-key` を指定すると、レプリケーションの待ち受けも同じ証明書で TLS にする（平文のフォロワーは接続できない）
- [x] フォロワーに `--leader-ca <PATH>`（環境変数 `MAHARIT_LEADER_CA`）を追加。`--leader-addr` のホスト名で証明書を検証して TLS で接続する
- [x] シークレットは TLS の中でだけ送られる。TLS なしでリーダーがループバック以外に待ち受けると警告
- [x] テスト: ユニット `replication_over_tls`、E2E `scripts/tls_test.py` に TLS のリーダー + フォロワー 3 種（検証あり / 平文 / 別の CA）
- [x] docs: advanced/replication.md（存在しないフラグ・機能の記述を実装に合わせて書き直し）、operations/tls.md、server-config.md
- コーディネーター↔シャードの TLS は server/151 に分けた

## 実施メモ（2026-11-08）
- 調べる中で、昇格コマンドが機能しない・フォロワーが書き込みを受け付ける・既存データのスナップショット同期が無い、を見つけた（bug/152）。docs には制限として記載
- Python クライアントの `stats()` が `replication` を捨てていたので返すようにした
