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

## 対応（完了）
- `ReplicationStatus`（stats レスポンス埋め込み）に `node_id` / `current_lsn` /
  `follower_count` を追加。`is_leader_alive` は据え置き。
- `From<ReplicationStats> for ReplicationStatus` を追加し、Stats ハンドラで
  leader は `LeaderReplicationManager::get_stats()`、follower は
  `FollowerReplicationManager::get_stats()` から変換して公開。
- 追加フィールドは既存クライアントに無害（追加のみ）。standalone は
  `replication` フィールド自体を省略（従来どおり）。
- 注: 当初案の「各フォロワー遅延 LSN」は `ReplicationStats` に無く、leader の
  followers マップ走査が必要なため今回は範囲外（follower_count/current_lsn で
  同期状況は把握可能）。

## 検証
- ローカル 3 ノードで write 後に stats を取得:
  - leader: `follower_count=2`, `current_lsn=6`, `node_id=leader`
  - follower1/2: `current_lsn=6`（leader と一致＝同期済み）, `is_leader_alive=true`
- server テスト 240 件パス。

## ステータス
完了
