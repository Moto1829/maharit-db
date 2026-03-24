---
title: シャーディング
parent: 高度なトピック
nav_order: 3
---

# シャーディング（maharit-cluster）

maharit-cluster クレートは、グラフデータを複数のシャード（ノード）に分散して管理するシャーディング機能を提供します。

## 概要

シャーディングにより、1 台のサーバーに収まらない大規模グラフを複数のサーバーに分散できます。

```
クライアント
    │
    ▼
QueryRouter（クエリをどのシャードに送るか決定）
    │
    ├── Shard 0 (例: NodeId 0–999)
    ├── Shard 1 (例: NodeId 1000–1999)
    └── Shard 2 (例: NodeId 2000–2999)
```

## シャーディング戦略

`ShardingStrategy` トレイトを実装した 3 つの戦略が用意されています。

### HashSharding（ハッシュシャーディング）

NodeId のハッシュ値を使ってシャードを決定します。データを均等に分散させるのに適しています。

```rust
use maharit_cluster::HashSharding;

let strategy = HashSharding::new(3); // シャード数 = 3
let shard_id = strategy.shard_for_node(node_id);
```

### RangeSharding（レンジシャーディング）

NodeId の範囲に基づいてシャードを決定します。連続した ID を持つノードを同じシャードにまとめたい場合に適しています。

```rust
use maharit_cluster::RangeSharding;

let strategy = RangeSharding::new(vec![0, 1000, 2000]); // 各シャードの開始 ID
let shard_id = strategy.shard_for_node(node_id);
```

### LabelSharding（ラベルシャーディング）

ノードのラベルに基づいてシャードを決定します。ラベル単位でデータを分離したい場合に適しています。

```rust
use maharit_cluster::LabelSharding;
use std::collections::HashMap;

let mut label_map = HashMap::new();
label_map.insert("Person".to_string(), 0);
label_map.insert("Product".to_string(), 1);
label_map.insert("Order".to_string(), 2);

let strategy = LabelSharding::new(label_map, 0); // デフォルトシャード = 0
```

## シャードマップ

`ShardMap` はクラスター内のすべてのシャード情報を管理します。

```rust
use maharit_cluster::{ShardMap, ShardInfo, ShardStatus};

let mut map = ShardMap::new();

// シャードを追加
map.add_shard(ShardInfo {
    id: 0,
    address: "127.0.0.1:7687".to_string(),
    status: ShardStatus::Active,
    node_count: 0,
    edge_count: 0,
});

// シャードの取得
let shard = map.get_shard(0).unwrap();
println!("Shard 0 address: {}", shard.address);

// アクティブなシャード一覧
let active = map.active_shards();
```

### シャードステータス

| ステータス | 説明 |
|-----------|------|
| `Active` | 通常稼働中 |
| `Inactive` | 停止中 |
| `Rebalancing` | データ移動中 |

## クエリルーティング

`QueryRouter` はクエリをどのシャードに転送するかを決定します。

```rust
use maharit_cluster::{QueryRouter, HashSharding, ShardMap};

let strategy = HashSharding::new(3);
let router = QueryRouter::new(strategy, shard_map);

// ノード ID に基づいてルーティング先を決定
let decision = router.route_for_node(node_id);
```

### RoutingDecision

| バリアント | 説明 |
|-----------|------|
| `SingleShard(id)` | 特定のシャードのみに送る |
| `AllShards` | 全シャードにブロードキャスト（例: MATCH (n) RETURN n） |
| `MultiShard(ids)` | 複数のシャードに送る |

## クラスターコーディネーター

`ClusterCoordinator` は複数のシャードへのクエリ実行と結果のマージを担当します。

```rust
use maharit_cluster::{ClusterCoordinator, ClusterConfig};

let config = ClusterConfig {
    shards: vec![
        ShardConfig { id: 0, address: "127.0.0.1:7687".to_string() },
        ShardConfig { id: 1, address: "127.0.0.1:7688".to_string() },
    ],
    replication_factor: 1,
};

let coordinator = ClusterCoordinator::new(config);

// 全シャードに対してクエリを実行し、結果をマージ
let rows = coordinator.execute_all("MATCH (n:Person) RETURN n.name").await?;
```

## リバランス（データ移動）

シャード間のデータ偏りを解消するためにリバランスを実行できます。

```rust
use maharit_cluster::{ShardMap, RebalancePlan};

let plan = shard_map.plan_rebalance();
for mv in &plan.moves {
    println!(
        "Node {} を Shard {} → Shard {} に移動",
        mv.node_id, mv.from_shard, mv.to_shard
    );
}
```

## エッジのルーティング

クロスシャードエッジ（異なるシャードにまたがるエッジ）の扱いには注意が必要です。

```rust
use maharit_cluster::classify_edge;

let location = classify_edge(from_shard_id, to_shard_id);
// EdgeLocation::Local   – 同じシャード内
// EdgeLocation::Remote  – 異なるシャードにまたがる
```

クロスシャードエッジは両側のシャードに参照を保持し、整合性を維持します。

## 設計上の注意

- **トランザクション境界**: 現在、分散トランザクション（2フェーズコミット）はサポートしていません。単一シャード内のトランザクションのみ保証されます。
- **クロスシャードクエリ**: `MATCH` + `JOIN` 相当のクエリは全シャードに問い合わせて結果をマージします。
- **シャード数の変更**: シャード数を変更する場合はリバランスが必要です。
