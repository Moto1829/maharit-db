# シャーディング

**Status**: Completed

## 概要
データを複数のノードに分散配置し、水平スケーラビリティを実現する。

## 実装内容

### シャーディング戦略
- [x] ハッシュベースシャーディング
- [x] レンジベースシャーディング
- [x] ラベルベースシャーディング

### シャード管理
- [x] シャードの追加/削除
- [x] リバランシング
- [x] シャードマッピングの管理

### 分散クエリ実行
- [x] クエリルーティング
- [x] 分散結合（Distributed Join）
- [x] 結果のマージ

### コーディネーター
- [x] クエリの分解
- [x] シャードへの分散
- [x] 結果の集約

### エッジの扱い
- [x] ローカルエッジ（同一シャード内）
- [x] リモートエッジ（シャード間）
- [x] エッジのレプリケーション戦略

## 設定例
```toml
[sharding]
enabled = true
strategy = "hash"
shards = [
  { id = 1, address = "node1:7687" },
  { id = 2, address = "node2:7687" },
  { id = 3, address = "node3:7687" },
]
replication_factor = 2
```

## 依存
- `19-replication.md` が完了していること

## 対象クレート
新規 `maharit-cluster` クレートとして実装 (`crates/maharit-cluster/`)
