---
title: Sharding
parent: Advanced topics
nav_order: 3
source_commit: e3173cc2
---

# Sharding

Sharding splits a graph too large for one machine across several servers (shards). There are two parts:

- **The coordinator server** (`maharit server --coordinator`): sends read queries from clients to every shard and merges the results into one
- **The `maharit-cluster` crate** (Rust API): strategies that decide which shard a node goes to, and management of the list of shards

Each shard is a regular `maharit server`. The application decides which data goes to which shard and writes to the shards directly (the `maharit-cluster` strategies can compute where each node goes).

```
Client ──reads──▶ Coordinator ──▶ Shard 0, Shard 1, … (the same query on every shard)
Application ──writes──▶ The target shard (chosen by a strategy)
```

## The coordinator server (`maharit server --coordinator`)

The coordinator sends queries from clients to every shard (each a regular `maharit server`) and merges the results.

```bash
# Shard (TLS + authentication)
maharit server --host 0.0.0.0 --port 7687 --data /var/lib/maharit/shard0.db \
  --tls-cert /etc/maharit/shard0.crt --tls-key /etc/maharit/shard0.key \
  --require-auth --admin-password "$SHARD_PASSWORD"

# Coordinator
maharit server --coordinator --host 0.0.0.0 --coordinator-port 7690 \
  --shards shard0.example.com:7687,shard1.example.com:7687 \
  --tls-cert /etc/maharit/coord.crt --tls-key /etc/maharit/coord.key \
  --shard-ca /etc/maharit/ca.crt \
  --shard-user admin --shard-password "$SHARD_PASSWORD" \
  --require-auth --admin-password "$COORD_PASSWORD"
```

- `--tls-cert` / `--tls-key`: use TLS for connections from clients to the coordinator ([TLS/SSL](../operations/tls.md))
- `--shard-ca`: verify the shards' certificates and connect over TLS. Each shard's certificate must be valid for the host name (or IP address) given in `--shards`
- `--shard-user` / `--shard-password`: log in to shards running with `--require-auth`. When a session expires, the coordinator logs in again automatically
- `--require-auth` / `--admin-password`: login to the coordinator itself (separate from the shard credentials)
- If any shard returns an error or cannot be reached, the whole query fails

### Queries the coordinator can run

The coordinator is **read only**. It runs the same query on every shard and merges the rows so that the result is the same as if all the shards' data were one database.

- **Writes** (`CREATE` / `MERGE` / `SET` / `DELETE`, creating indexes and constraints, and so on) fail. Write data by connecting directly to the shard where it should live
- **Aggregation**: `count` / `sum` / `min` / `max` / `collect` combine the values from every shard per group (the values of the non-aggregated columns)
- **`ORDER BY` / `SKIP` / `LIMIT` / `RETURN DISTINCT`** apply to the rows from all shards. `ORDER BY` may only refer to returned columns (or their aliases)
- Rows with the same values that come from different nodes are all returned
- `UNION` concatenates the rows from all shards; `UNION` (without `ALL`) collapses identical rows into one
- Queries that do not read the graph (such as `RETURN 1`) and `EXPLAIN` run on the first shard only
- `SHOW CONSTRAINTS` / `SHOW INDEXES` / procedure calls concatenate the rows from all shards and collapse identical rows

These queries fail, because a correct overall result cannot be built from per-shard results:

- Aggregates such as `avg` / `stDev` / `percentileCont`, and aggregates with `DISTINCT` such as `count(DISTINCT …)`
- Aggregation, `DISTINCT`, `SKIP`, or `LIMIT` in `WITH`; `CALL { … }` subqueries; `RETURN *`; `PROFILE`
- `SKIP` / `LIMIT` with anything other than an integer literal (parameters or expressions; the coordinator does not accept parameters)

!!! warning "Patterns across shards"
    Each shard matches patterns against its own data only. A relationship whose endpoints are on different shards cannot be created or traversed.

## Rust API (`maharit-cluster`)

### Sharding strategies

`shard_for_node(node_id, label)` of the `ShardingStrategy` trait returns the ID of the shard a node goes to. There are three strategies.

```rust
use maharit_cluster::{HashSharding, LabelSharding, RangeSharding, ShardingStrategy};

// Spread evenly by a hash of the node ID (3 shards)
let hash = HashSharding::new(3);
let shard = hash.shard_for_node(42, "Person");

// Split by node ID range (ranges include both ends; anything else goes to default shard 0)
let mut range = RangeSharding::new(0);
range.add_range(0, 999, 0);
range.add_range(1000, 1999, 1);
let shard = range.shard_for_node(1500, "Person"); // 1

// Split by label (unregistered labels go to default shard 0)
let mut label = LabelSharding::new(0);
label.add_label("Person", 0);
label.add_label("Product", 1);
let shard = label.shard_for_node(42, "Product"); // 1
```

### Shard map and routing

`ShardMap` holds the list of shards (ID, address, status, node count), and `QueryRouter` decides which shards to query from a strategy and the shard map.

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
let shard = router.route_node_query(42, "Person");   // The shard a node goes to
let all = router.route_all();                        // Every active shard
match router.decide_for_label("Person") {
    RoutingDecision::SingleShard(id) => println!("query shard {id} only"),
    RoutingDecision::MultiShard(ids) => println!("query {ids:?}"),
    RoutingDecision::Broadcast => println!("query every shard"),
}
```

| Shard status (`ShardStatus`) | Description |
|-----------|------|
| `Active` | Running |
| `Inactive` | Stopped |
| `Rebalancing` | Moving data |

### Planning a rebalance

`ShardMap::rebalance_plan()` produces a list of moves that even out node counts, based on each shard's `node_count`. It only plans; no data is moved.

```rust
let plan = map.rebalance_plan();
for mv in &plan.moves {
    println!("move node {} from shard {} to shard {}", mv.node_id, mv.from_shard, mv.to_shard);
}
```

### Where a relationship's endpoints live

`classify_edge` tells whether both endpoint nodes of a relationship go to the same shard.

```rust
use maharit_cluster::{EdgeLocation, classify_edge};

match classify_edge(1, 2, &router, "Person", "Company") {
    EdgeLocation::Local(shard) => println!("both ends on shard {shard}"),
    EdgeLocation::Remote { src, dst } => println!("spans shard {src} and shard {dst}"),
}
```

## Design notes

- **Relationships across shards**: a shard does not know about other shards' nodes, so a relationship whose endpoints are on different shards cannot be created. Keep data that is traversed together on the same shard (for example by placing it with `LabelSharding`)
- **Transactions**: there are no transactions across shards (two-phase commit). A transaction covers a single shard only
- **Changing the number of shards**: placement changes, so data has to be moved again (plan the moves with `rebalance_plan()`; moving is manual)
