# Task 19: レプリケーション

## 概要
Add leader/follower replication to maharit-server.

## ステータス
- [x] Completed

## 実装内容

### File
`crates/maharit-server/src/replication.rs`

### Architecture
- `NodeRole`: Leader or Follower enum
- `ReplicationConfig`: configuration for replication (role, node_id, bind_address, leader_address, heartbeat settings)
- `WalEntryData`: WAL entry variants (CreateNode, DeleteNode, CreateEdge, DeleteEdge, SetProperty)
- `ReplicationMessage`: protocol messages (Handshake, HandshakeAck, Heartbeat, HeartbeatAck, WalEntry, WalAck, Shutdown)
- `LeaderReplicationManager`: manages follower connections, broadcasts WAL entries via tokio broadcast channel
- `FollowerReplicationManager`: connects to leader, receives WAL entries, tracks leader health
- `ReplicationError`: custom error type using thiserror

### Protocol
- Length-prefixed JSON over TCP (same pattern as tcp_server.rs)
- Leader listens on replication port (default: 7688)
- Followers connect to leader and receive WAL entries

### Tests
9 unit tests covering config defaults, serialization, struct creation, stats, and LSN incrementing.
