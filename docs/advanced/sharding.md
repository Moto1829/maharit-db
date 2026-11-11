---
title: シャーディング
parent: 高度なトピック
nav_order: 3
---

# シャーディング

1 台に収まらないグラフを、複数のサーバー（シャード）に分けて置く仕組みです。次の 2 つの部品があります。

- **コーディネーターサーバー**（`maharit server --coordinator`）: クライアントからの読み取りクエリを全シャードに送り、結果を 1 つにまとめて返す
- **`maharit-cluster` クレート**（Rust API）: ノードをどのシャードに置くかを決める戦略と、シャードの一覧の管理

シャード自体は通常の `maharit server` です。どのデータをどのシャードに置くかはアプリケーションが決め、シャードに直接書き込みます（`maharit-cluster` の戦略で置き先を計算できます）。

```
クライアント ──読み取り──▶ コーディネーター ──▶ Shard 0, Shard 1, …（全シャードに同じクエリ）
アプリケーション ──書き込み──▶ 置き先のシャード（戦略で決める）
```

## コーディネーターサーバー（`maharit server --coordinator`）

コーディネーターは、クライアントからのクエリを全シャード（通常の `maharit server`）に送り、結果を合わせて返します。

```bash
# シャード（TLS + 認証）
maharit server --host 0.0.0.0 --port 7687 --data /var/lib/maharit/shard0.db \
  --tls-cert /etc/maharit/shard0.crt --tls-key /etc/maharit/shard0.key \
  --require-auth --admin-password "$SHARD_PASSWORD"

# コーディネーター
maharit server --coordinator --host 0.0.0.0 --coordinator-port 7690 \
  --shards shard0.example.com:7687,shard1.example.com:7687 \
  --tls-cert /etc/maharit/coord.crt --tls-key /etc/maharit/coord.key \
  --shard-ca /etc/maharit/ca.crt \
  --shard-user admin --shard-password "$SHARD_PASSWORD" \
  --require-auth --admin-password "$COORD_PASSWORD"
```

- `--tls-cert` / `--tls-key`: クライアントからコーディネーターへの接続を TLS にする（[TLS/SSL 設定](../operations/tls.md)）
- `--shard-ca`: シャードの証明書を検証して TLS で接続する。各シャードの証明書は、`--shards` に書いたホスト名（または IP アドレス）に対して有効である必要がある
- `--shard-user` / `--shard-password`: `--require-auth` のシャードにログインする。セッションが切れたら自動でログインし直す
- `--require-auth` / `--admin-password`: コーディネーター自身へのログイン（シャードの資格情報とは別）
- どれかのシャードがエラーを返したり接続できなかったりすると、クエリ全体がエラーになる

### コーディネーターで実行できるクエリ

コーディネーターは**読み取り専用**です。同じクエリを全シャードで実行し、全シャードのデータを 1 つのデータベースとみなしたときと同じ結果になるように、行をまとめて返します。

- **書き込み**（`CREATE` / `MERGE` / `SET` / `DELETE`、インデックス・制約の作成など）はエラーになります。データは、置きたいシャードに直接接続して書き込んでください
- **集計**: `count` / `sum` / `min` / `max` / `collect` は、グループ（集計でない列の値）ごとに全シャードの値を合算します
- **`ORDER BY` / `SKIP` / `LIMIT` / `RETURN DISTINCT`** は全シャードの行に対して適用します。`ORDER BY` には `RETURN` した列（またはその別名）だけを書けます
- 同じ値の行も、別のノードから来たものはそのまま返します
- `UNION` は全シャードの行を連結し、`UNION`（`ALL` なし）では同じ行を 1 行にします
- グラフを読まないクエリ（`RETURN 1` など）と `EXPLAIN` は、最初のシャードだけで実行します
- `SHOW CONSTRAINTS` / `SHOW INDEXES` / プロシージャ呼び出しは、全シャードの行を連結して同じ行を 1 行にします

次のクエリは、シャードごとの結果から正しい全体の結果を作れないため、エラーになります。

- `avg` / `stDev` / `percentileCont` などの集計と、`count(DISTINCT …)` などの `DISTINCT` 付き集計
- `WITH` での集計・`DISTINCT`・`SKIP`・`LIMIT`、`CALL { … }` サブクエリ、`RETURN *`、`PROFILE`
- `SKIP` / `LIMIT` に整数リテラル以外（パラメータや式）を使うもの（コーディネーターはパラメータを受け取りません）

!!! warning "シャードをまたぐパターン"
    各シャードは自分のデータだけでパターンを照合します。両端が別のシャードにあるリレーションシップは作れず、辿ることもできません。

## Rust API（`maharit-cluster`）

### シャーディング戦略

`ShardingStrategy` トレイトの `shard_for_node(node_id, label)` が、ノードを置くシャードの ID を返します。3 つの戦略があります。

```rust
use maharit_cluster::{HashSharding, LabelSharding, RangeSharding, ShardingStrategy};

// ノード ID のハッシュで均等に分ける（シャード数 = 3）
let hash = HashSharding::new(3);
let shard = hash.shard_for_node(42, "Person");

// ノード ID の範囲で分ける（範囲は両端を含む。どれにも入らなければ既定のシャード 0）
let mut range = RangeSharding::new(0);
range.add_range(0, 999, 0);
range.add_range(1000, 1999, 1);
let shard = range.shard_for_node(1500, "Person"); // 1

// ラベルで分ける（登録の無いラベルは既定のシャード 0）
let mut label = LabelSharding::new(0);
label.add_label("Person", 0);
label.add_label("Product", 1);
let shard = label.shard_for_node(42, "Product"); // 1
```

### シャードマップとルーティング

`ShardMap` はシャードの一覧（ID・アドレス・状態・ノード数）を持ち、`QueryRouter` は戦略とシャードマップから問い合わせ先を決めます。

```rust
use std::sync::{Arc, RwLock};
use maharit_cluster::{
    HashSharding, QueryRouter, RoutingDecision, ShardInfo, ShardMap, ShardStatus,
};

let mut map = ShardMap::new();
for (id, address) in [(0, "10.0.0.1:7687"), (1, "10.0.0.2:7687")] {
    map.add_shard(ShardInfo {
        id,
        address: address.to_string(),
        status: ShardStatus::Active,
        node_count: 0,
    })?;
}

let router = QueryRouter::new(Arc::new(RwLock::new(map)), Box::new(HashSharding::new(2)));
let shard = router.route_node_query(42, "Person");   // ノードを置くシャード
let all = router.route_all();                        // 稼働中の全シャード
match router.decide_for_label("Person") {
    RoutingDecision::SingleShard(id) => println!("shard {id} だけに問い合わせる"),
    RoutingDecision::MultiShard(ids) => println!("{ids:?} に問い合わせる"),
    RoutingDecision::Broadcast => println!("全シャードに問い合わせる"),
}
```

| シャードの状態（`ShardStatus`） | 説明 |
|-----------|------|
| `Active` | 稼働中 |
| `Inactive` | 停止中 |
| `Rebalancing` | データ移動中 |

### リバランスの計画

`ShardMap::rebalance_plan()` は、各シャードの `node_count` から、ノード数を均すための移動の一覧を作ります。計画を作るだけで、データは移動しません。

```rust
let plan = map.rebalance_plan();
for mv in &plan.moves {
    println!("ノード {} を shard {} → shard {} に移す", mv.node_id, mv.from_shard, mv.to_shard);
}
```

### リレーションシップの両端の位置

`classify_edge` は、リレーションシップの両端のノードが同じシャードに置かれるかを判定します。

```rust
use maharit_cluster::{EdgeLocation, classify_edge};

match classify_edge(1, 2, &router, "Person", "Company") {
    EdgeLocation::Local(shard) => println!("両端が shard {shard}"),
    EdgeLocation::Remote { src, dst } => println!("shard {src} と shard {dst} にまたがる"),
}
```

## 設計上の注意

- **シャードをまたぐリレーションシップ**: シャードは他のシャードのノードを知らないため、両端が別のシャードにあるリレーションシップは作れません。一緒に辿るデータは、同じシャードに置くようにしてください（`LabelSharding` などで置き先をそろえる）
- **トランザクション**: 複数のシャードにまたがるトランザクション（2 フェーズコミット）はありません。トランザクションは 1 つのシャードの中だけです
- **シャード数の変更**: 置き先の計算が変わるため、データを移し直す必要があります（移動の計画は `rebalance_plan()`、移動は手作業）
