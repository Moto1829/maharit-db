# Task 69: シャーディングを TcpServer に統合する

## 概要

`maharit-cluster` クレートの `ClusterCoordinator` / `ShardingStrategy` を
`maharit-server` の TCP サーバーに統合し、シャードノードとコーディネーターモードで
起動できるようにする。

## 実装ファイル

| ファイル | 変更内容 |
|---|---|
| `crates/maharit-cluster/Cargo.toml` | `serde_json` 依存を追加 |
| `crates/maharit-cluster/src/shard_client.rs` | 新規: TCP シャードクライアント |
| `crates/maharit-cluster/src/lib.rs` | `shard_client` モジュール公開 |
| `crates/maharit-server/Cargo.toml` | `maharit-cluster` 依存を追加 |
| `crates/maharit-server/src/coordinator.rs` | 新規: ShardCoordinatorServer |
| `crates/maharit-server/src/main.rs` | シャード/コーディネーターフラグを追加 |

## 新しい CLI フラグ

```
maharit server --shard --shard-id 0 --port 7687
maharit server --shard --shard-id 1 --port 7688
maharit server --coordinator \
  --shards 127.0.0.1:7687,127.0.0.1:7688 \
  --coordinator-port 7690
```

## ステータス

- [x] 完了
