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
- [ ] Cargo featureフラグでレプリケーション機能を有効/無効化
- [ ] 設定ベースの有効化（`replication: Option<ReplicationConfig>`）
- [ ] トレイトによるStorageBackend抽象化（InMemory, SingleNode, Replicated）
- [ ] 既存APIとの後方互換性を維持

## 実装内容

### リーダー/フォロワー構成
- [ ] リーダーノードの選出
- [ ] フォロワーノードの登録/解除
- [ ] ノード間の接続管理
- [ ] ヘルスチェック（ハートビート）

### WALストリーミング
- [ ] WALエントリのシリアライズ
- [ ] リーダーからフォロワーへのWAL転送
- [ ] フォロワーでのWAL適用
- [ ] 同期/非同期レプリケーションモード

### 自動フェイルオーバー
- [ ] リーダー障害の検出
- [ ] フォロワーの昇格（リーダー選出）
- [ ] クライアントへの新リーダー通知
- [ ] スプリットブレイン防止

### 読み取りスケールアウト
- [ ] 読み取りクエリのフォロワーへのルーティング
- [ ] ロードバランシング（ラウンドロビン等）
- [ ] レプリケーションラグの監視
- [ ] 読み取り一貫性レベル（eventual/strong）

### 設定
- [ ] レプリケーション設定ファイル
- [ ] ノードロール設定（leader/follower/candidate）
- [ ] 同期レプリカ数の設定
- [ ] タイムアウト設定

## API例
```rust
// オンメモリ（従来通り）
let graph = Graph::new();

// 単一ノード（従来通り）
let server = TcpServer::new(config);

// リーダーノード（レプリケーション有効）
let config = ReplicationConfig {
    role: NodeRole::Leader,
    bind_address: "0.0.0.0:7688",
    ..Default::default()
};
let server = TcpServer::with_replication(config);

// フォロワーノード
let config = ReplicationConfig {
    role: NodeRole::Follower,
    leader_address: "leader:7688",
    ..Default::default()
};
let server = TcpServer::with_replication(config);

// クライアント（読み取りスケールアウト）
let client = Client::connect_cluster(&["node1:7687", "node2:7687", "node3:7687"]).await?;
```

## 依存
- `10-wal.md` が完了していること
- `12-tcp-server.md` が完了していること

## 対象クレート
新規 `maharit-replication` または `maharit-server` に追加
