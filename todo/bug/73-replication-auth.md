# bug/73: レプリケーションチャネルの共有シークレット認証

## 概要
`handle_follower_connection` のハンドシェイクは認証が皆無で、
レプリケーションポートに到達できる任意のクライアントが `current_lsn: 0` を
送るだけで**グラフ全体のスナップショットと以降の全 WAL 更新を受信**できた。
無認証のデータ全件エクスフィルトレーション (CRITICAL)。

さらに `recv_message` は u32 長プレフィックスを上限なく `vec![0u8; len]` で
確保しており、巨大な長さ宣言でメモリを枯渇させられた。

## 対応
- `ReplicationConfig.shared_secret: Option<String>` を追加。
- `ReplicationMessage::Handshake` に `auth_token: Option<String>` を追加。
- `ReplicationMessage::Unauthorized { reason }` を追加。
- リーダー側: シークレット設定時、`auth_token` が一致しないフォロワーを
  スナップショット送信前に拒否（定数時間比較 `replication_secret_eq`）。
- フォロワー側: ハンドシェイクに設定済みシークレットを付与し、
  `Unauthorized` を受けたら接続失敗として扱う。
- `main.rs` に `--replication-secret` / `MAHARIT_REPLICATION_SECRET` を追加し
  リーダー・フォロワー両設定へ配線。
- `recv_message` に `MAX_REPLICATION_MESSAGE_SIZE`(1 GiB) の上限を追加。
- テスト 4 件追加（正しい/誤り/欠落シークレット + 定数時間比較）。

## 影響
- 重大度: CRITICAL
- `shared_secret` 未設定時は従来どおり無認証（信頼ネットワーク前提）。既存テスト・
  デプロイの互換性を維持しつつ、シークレット設定で強制認証を有効化できる。

## ステータス
完了（server テスト 231 件パス）
