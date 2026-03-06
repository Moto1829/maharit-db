# レプリケーション: フェイルオーバーコマンド

**Status**: Completed

## 概要

ドキュメント（`advanced/replication.md`）に記載されている
`maharit admin promote-to-leader` コマンドを実装する。
現状は `admin` サブコマンド自体が存在しない。

## 実装内容

- [x] CLI に `admin` サブコマンドを追加
- [x] `admin promote-to-leader --addr <follower-addr>` を実装
  - 対象フォロワーに `PromoteToLeader` メッセージを送信
  - フォロワーが受け取ったら自身のロールを Leader に切り替える
  - 旧リーダーに `Shutdown` を送信（または手動停止）
- [x] `ReplicationMessage::PromoteToLeader` を追加
- [x] フォロワーがリーダーに昇格する際の状態遷移を実装
  - `FollowerReplicationManager` → `LeaderReplicationManager` への切り替え
  - リプリケーションポートのリッスン開始

## 注意

自動フェイルオーバー（ハートビートタイムアウト時の自動昇格）はスコープ外。
手動フェイルオーバーのみ実装する。

## 依存

- `51-replication-wal-apply.md` が完了していること
- `52-replication-tcp-integration.md` が完了していること

## 対象クレート

`maharit-server`
