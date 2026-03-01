# レプリケーション: TCP サーバーとの統合

## 概要

クエリ実行（CREATE / SET / DELETE）が `LeaderReplicationManager::append_wal_entry()` を
呼び出すよう接続する。現状はレプリケーションマネージャーが独立しており、
実際の書き込みがフォロワーに流れない。

## 現状の問題

- `tcp_server.rs` のクエリ実行パスが `append_wal_entry()` を呼んでいない
- リーダー/フォロワーの起動フラグがサーバー起動時に考慮されていない可能性がある

## 実装内容

- [ ] `TcpServer` に `Option<Arc<LeaderReplicationManager>>` を持たせる
- [ ] クエリ実行後、変更種別に応じた `WalEntryData` を生成して `append_wal_entry()` を呼ぶ
  - CREATE ノード → `WalEntryData::CreateNode`
  - CREATE エッジ → `WalEntryData::CreateEdge`
  - DELETE → `WalEntryData::DeleteNode` / `DeleteEdge`
  - SET プロパティ → `WalEntryData::SetProperty`
- [ ] `--enable-replication` / `--replication-role` フラグでサーバー起動時に
  `LeaderReplicationManager` または `FollowerReplicationManager` を初期化・起動する
- [ ] フォロワーへの書き込みリクエストをリーダーに転送する（または拒否してエラーを返す）

## 依存

- `51-replication-wal-apply.md` が完了していること

## 対象クレート

`maharit-server`
