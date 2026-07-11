# server/94: レプリケーション統計の拡充（stats に LSN・フォロワー数・遅延を公開）

## 背景
bug/90 で `stats` レスポンスに `replication.{role, is_leader_alive}` を追加し
failover 検知を可能にした。監視性をさらに高めるため、`ReplicationStats`
（`replication.rs` に既存: current_lsn / follower_count 等）を stats に配線したい。

## やること
- `Response::Stats.replication`（`ReplicationStatus`）に以下を追加:
  - `current_lsn`（leader/follower 双方）
  - `follower_count`（leader）
  - フォロワー遅延 LSN（leader が保持する各 FollowerState.last_lsn との差）
  - 最終ハートビート経過（follower）
- `LeaderReplicationManager` / `FollowerReplicationManager` の `get_stats()` を
  `TcpServer` の stats ハンドラから参照して配線（follower は既に `with_follower` で保持、
  leader は `replication` フィールドで保持）。
- 後方互換: 追加フィールドは `#[serde(skip_serializing_if)]` / default で任意化。

## 受け入れ条件
- leader の stats に current_lsn / follower_count が出る。
- follower の stats に current_lsn / is_leader_alive が出る。
- 既存クライアント（追加フィールドを無視）が壊れない。

## 優先度 / 規模
- 中（監視性向上）。小〜中規模。bug/90 の自然な延長。

## ステータス
未着手（バックログ）
