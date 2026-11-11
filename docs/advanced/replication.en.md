---
title: Replication
parent: Advanced topics
nav_order: 2
source_commit: eefb1a37
---

# Replication

MaharitDB supports leader/follower replication based on WAL streaming.

## Architecture

```
[Leader]
  ├── Receives writes from clients
  ├── Records changes in the WAL
  └── Streams WAL entries to followers

[Follower 1]     [Follower 2]
  ├── On connect, receives a snapshot and replaces its graph
  ├── Then receives and applies the WAL
  └── Read only (writes are rejected)
```

## Setup

### Leader

```bash
maharit server \
  --host 0.0.0.0 --port 7687 \
  --replication-role leader \
  --replication-bind 0.0.0.0:7688 \
  --replication-secret "$MAHARIT_REPLICATION_SECRET" \
  --node-id leader
```

### Follower

```bash
maharit server \
  --host 0.0.0.0 --port 7687 \
  --replication-role follower \
  --leader-addr leader-host:7688 \
  --replication-secret "$MAHARIT_REPLICATION_SECRET" \
  --node-id follower-1
```

`--node-id` must be unique within the cluster. To run several followers on one host, give each a different `--port`, `--node-id`, and `--metrics-bind`.

| Option | Applies to | Description |
|-----------|------|------|
| `--replication-role` | Both | `leader` / `follower` |
| `--replication-bind` | Both | Replication listen address (default `127.0.0.1:7688`). A follower uses it after promotion |
| `--leader-addr` | Follower | The leader's replication address (required) |
| `--replication-secret` | Both | Shared secret (env: `MAHARIT_REPLICATION_SECRET`). When set, the leader rejects followers that do not present the same secret |
| `--leader-ca` | Follower | CA certificate to verify the leader with; connects over TLS (env: `MAHARIT_LEADER_CA`) |
| `--node-id` | Both | Node ID (env: `MAHARIT_NODE_ID`, default `node-1`) |

## Encryption (TLS)

Start the leader with `--tls-cert` / `--tls-key` and the replication connection (7688) uses TLS in addition to client connections (7687); plaintext followers can no longer connect. Give followers `--leader-ca`, the CA certificate that verifies the leader's certificate (the leader's own certificate when self-signed).

```bash
# Leader
maharit server --host 0.0.0.0 \
  --tls-cert /etc/maharit/leader.crt --tls-key /etc/maharit/leader.key \
  --replication-role leader --replication-bind 0.0.0.0:7688 \
  --replication-secret "$MAHARIT_REPLICATION_SECRET"

# Follower
maharit server --host 0.0.0.0 \
  --tls-cert /etc/maharit/follower.crt --tls-key /etc/maharit/follower.key \
  --replication-role follower --leader-addr leader.example.com:7688 \
  --leader-ca /etc/maharit/ca.crt \
  --replication-secret "$MAHARIT_REPLICATION_SECRET"
```

- The leader's certificate must be valid for the host name (or IP address) in the followers' `--leader-addr`.
- The shared secret is sent inside TLS. Without TLS, the secret and the replicated data cross the network in plaintext, and anyone who captures them can impersonate a follower. A leader that listens on a non-loopback address without TLS logs a warning at startup.
- A follower's own `--tls-cert` / `--tls-key` encrypt client connections to that follower (optional).

## WAL streaming

### How streaming works

The leader streams committed WAL entries to followers asynchronously.

```
Leader WAL:
  LSN 1001: CREATE (n:Person {name: "Alice"})
  LSN 1002: SET alice.age = 30
  LSN 1003: CREATE (a)-[:KNOWS]->(b)

Follower (receives and applies in real time):
  LSN 1001: applied
  LSN 1002: applied
  LSN 1003: receiving...
```

### Checking replication status

The `replication` field of each node's `stats` reply (`client.stats()` in Python) holds the role, node ID, the applied LSN, the number of connected followers (leader), and whether the leader is alive (follower). The difference between the leader's and a follower's `current_lsn` is the number of changes not yet applied.

```python
from maharit import Client

with Client.connect("follower-1:7687") as client:
    print(client.stats()["replication"])
    # {'role': 'follower', 'node_id': 'follower-1', 'current_lsn': 1003,
    #  'follower_count': 0, 'is_leader_alive': True}
```

## Heartbeats and liveness

The leader sends a heartbeat to followers every second. A follower that receives no heartbeat for 5 seconds considers the leader down (`is_leader_alive` becomes `false`). The intervals cannot be changed.

When the connection drops, the follower reconnects to the leader (`--leader-addr`) every second. Followers can be started before the leader.

## Reading from followers

Followers accept read queries only. Write queries get the error `this node is a read-only follower: send writes to the leader`; send writes to the leader.

```rust
// Connect to a follower and read
let mut follower = Client::connect("follower-1:7687").await?;
let result = follower.query("MATCH (n:Person) RETURN n.name").await?;

// Connect to the leader to write
let mut leader = Client::connect("leader:7687").await?;
leader.execute("CREATE (n:Person {name: \"Alice\"})").await?;
```

### Read consistency

Reads from followers are eventually consistent. With replication lag, a follower can return older data than the leader.

Read from the leader when you need strong consistency.

## Syncing existing data

Every time a follower connects (or reconnects) to the leader, the leader sends a snapshot of the whole graph at that moment, followed by every change after it. The snapshot is taken while writes are briefly paused, so there is no gap or overlap between the snapshot and the changes that follow.

On receiving the snapshot, the follower replaces its whole graph. Nothing from the follower's own data file or old data from while it was disconnected remains. You can add an empty follower to a leader that already has data.

- While the graph is being replaced (a few seconds for a large graph), reads from that follower can return an intermediate state.
- Definitions of indexes, constraints, and full-text indexes arrive with the snapshot, and later `CREATE` / `DROP` statements are replicated too. Each follower builds its own indexes and keeps them up to date with the replicated changes. You cannot run `CREATE INDEX` and the like directly on a follower (they are rejected as writes).

## Failover

When the leader goes down, promote one of the followers. `--addr` is that follower's **client** address.

```bash
maharit admin promote-to-leader --addr follower-1:7687
# TLS follower:                      --ca /etc/maharit/ca.crt
# Follower with --require-auth:      --password <admin password> (or MAHARIT_ADMIN_PASSWORD; --user for another admin)
```

The promoted follower switches over in this order:

1. Stops receiving from the old leader
2. Saves its current graph to the data file and starts using a WAL (later writes survive crashes)
3. Starts accepting follower connections on its own `--replication-bind` (default `127.0.0.1:7688`)
4. Starts accepting writes

Restart the remaining followers with `--leader-addr` pointing at the new leader's `--replication-bind`, and they catch up with a snapshot. If every follower's `--leader-addr` is a virtual IP or DNS name that reaches whichever node is leader, no restart is needed (followers reconnect automatically).

- Only administrators (the admin role with `--require-auth`) can send the promotion request. A second promotion is an error.
- A promoted node goes back to being a follower when restarted with its original flags. To keep it as the leader, start it with `--replication-role leader`.
- Bring the old leader back as a follower (`--replication-role follower --leader-addr <new leader>`). Its data is replaced by the new leader's snapshot. Starting the old leader as a leader leaves you with two leaders and split writes.
- Automatic failover (detecting a down leader and promoting automatically) is not supported. Monitor `is_leader_alive` in `stats` and run the promotion yourself.

## Summary

| Item | Leader | Follower |
|---------|--------|---------|
| Writes | Yes | Rejected (accepted after promotion) |
| Reads | Yes | Yes (with lag) |
| Heartbeats | Sends | Receives |
| WAL streaming | Sends | Receives and applies |
| Persistence | Data file + WAL | Saves the data file at shutdown (replaced by the leader's snapshot on connect). Uses a WAL after promotion |
| Ports | 7687 (clients), 7688 (replication) | 7687 (clients); also 7688 after promotion |
