# レプリケーション: フォロワーへの WAL 適用

**Status**: Completed

## 概要

フォロワーが受信した WAL エントリをローカルのグラフに実際に適用する。
現状は `entry: _` で捨てており、フォロワーのグラフにデータが反映されない。

## 現状の問題

`replication.rs` の受信ループが WAL エントリを無視している:

```rust
ReplicationMessage::WalEntry { lsn, entry: _ } => {
    // In a full implementation the entry would be applied to the
    // local graph here.  For now we just advance the LSN counter.
    current_lsn.store(lsn, Ordering::SeqCst);
```

## 実装内容

- [x] `FollowerReplicationManager` に `Arc<RwLock<Graph>>` を持たせる
- [x] `WalEntryData` の各バリアントに対応するグラフ操作を実装
  - `CreateNode` → `graph.create_node_with_id()`
  - `DeleteNode` → `graph.delete_node()`
  - `CreateEdge` → `graph.create_edge_with_id()`
  - `DeleteEdge` → `graph.delete_edge()`
  - `SetProperty` → `graph.set_node_property()` / `set_edge_property()`
- [x] `entry: _` → `entry` に変更し、適用ロジックを追加
- [x] 適用失敗時のエラーログを出力（適用はベストエフォートで続行）

## 依存

- `52-replication-tcp-integration.md` とセットで機能する

## 対象クレート

`maharit-server`
