# レプリケーション

## 概要
データの可用性とスケーラビリティを向上させるためのレプリケーション機能を実装する。

## 設計方針

### 互換性の維持
レプリケーションはオプショナル機能として実装し、既存の動作モードを維持する。

| モード | 永続化 | レプリケーション | 用途 |
|--------|--------|-----------------|------|
| オンメモリ | なし | なし | 開発、テスト |
| 単一ノード | WAL | なし | 小規模本番 |
| クラスタ | WAL | あり | 高可用性 |

### 実装アプローチ
- [x] 設定ベースの有効化（`ReplicationConfig`）
- [x] トレイトによるStorageBackend抽象化（InMemory, SingleNode, Replicated）

## 実装内容

### リーダー/フォロワー構成
- [x] リーダーノードの選出
- [x] フォロワーノードの登録/解除
- [x] ノード間の接続管理
- [x] ヘルスチェック（ハートビート）

### WALストリーミング
- [x] WALエントリのシリアライズ
- [x] リーダーからフォロワーへのWAL転送
- [x] フォロワーでのWAL適用
- [x] 同期/非同期レプリケーションモード

### 自動フェイルオーバー
- [x] リーダー障害の検出
- [x] フォロワーの昇格（リーダー選出）
- [x] クライアントへの新リーダー通知
- [x] スプリットブレイン防止

### 読み取りスケールアウト
- [x] レプリケーションラグの監視

### 設定
- [x] レプリケーション設定（`NodeRole`, `ReplicationConfig`）
- [x] ノードロール設定（leader/follower）
- [x] タイムアウト設定

## ステータス
✅ 完了 - `crates/maharit-server/src/replication.rs` に実装済み

### 実装した型
- `NodeRole` enum (Leader/Follower)
- `ReplicationConfig` 設定構造体
- `WalEntryData` - CreateNode/DeleteNode/CreateEdge/DeleteEdge/SetProperty
- `ReplicationMessage` - プロトコルメッセージ7種
- `LeaderReplicationManager` - リーダー側管理
- `FollowerReplicationManager` - フォロワー側管理
- `ReplicationStats` - 統計情報
- `ReplicationError` - エラー型

### テスト: 9件
