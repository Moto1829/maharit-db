# レプリケーション: WalEntryData の複数ラベル対応

**Status**: Completed

## 概要

タスク 48（複数ラベル）で `Node.labels: Vec<String>` になったが、
`WalEntryData::CreateNode` が `label: String` のままで整合していない。

## 現状の問題

```rust
// replication.rs
pub enum WalEntryData {
    CreateNode { node_id: u64, label: String },  // 旧: 単一ラベル
    ...
}
```

## 実装内容

- [x] `WalEntryData::CreateNode { label: String }` → `{ labels: Vec<String> }` に変更
- [x] `WalEntryData::CreateEdge` の `label: String` はエッジ型なので現状維持（エッジは単一型）
- [x] シリアライズ/デシリアライズの互換性を確認
- [x] 既存テスト (`test_wal_entry_data_variants`) を更新
- [x] `51-replication-wal-apply.md` の適用ロジックも `labels: Vec<String>` で実装すること

## 対象クレート

`maharit-server`
