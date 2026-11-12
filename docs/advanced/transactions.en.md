---
title: Transactions
parent: Advanced topics
nav_order: 1
source_commit: 6d1f9c68
---

# Transactions

With `maharit server`, you can group several queries into one transaction and then commit it, or roll it back to undo them all. Transactions are controlled not with Cypher statements (`BEGIN` / `COMMIT`) but with the `begin` / `commit` / `rollback` requests of the [TCP protocol](../api/protocol.md).

## From a client

### Python

```python
from maharit import Client

with Client.connect("127.0.0.1:7687") as client:
    # Commits when the block ends, rolls back on an exception
    with client.transaction() as tx:
        tx.execute("MATCH (a:Account {id: $id}) SET a.balance = a.balance - $amount",
                   {"id": "A", "amount": 100})
        tx.execute("MATCH (b:Account {id: $id}) SET b.balance = b.balance + $amount",
                   {"id": "B", "amount": 100})
```

`client.begin_transaction()` returns a `Transaction` on which you call `tx.commit()` / `tx.rollback()` yourself.

### Rust

```rust
let tx = client.begin().await?;
client.query_in_tx("MATCH (a:Account {id: 'A'}) SET a.balance = a.balance - 100", Some(tx)).await?;
client.query_in_tx("MATCH (b:Account {id: 'B'}) SET b.balance = b.balance + 100", Some(tx)).await?;
client.commit(tx).await?;      // client.rollback(tx) to undo
```

### Protocol

```json
{"type": "begin"}
{"type": "query", "query": "CREATE (:Item {id: $id})", "params": {"id": 1}, "txId": 7}
{"type": "commit", "txId": 7}
```

Put the `txId` from the `begin` reply `{"type": "transactionBegun", "txId": 7}` on the queries in the transaction and on `commit` / `rollback`.

## Behavior

Writes inside a transaction are **applied to the graph and the WAL when they run**. A rollback applies changes that cancel them out.

- **Visible to other connections**: uncommitted changes are visible to queries from other connections right away (equivalent to the Read Uncommitted isolation level). After a rollback they are gone
- **One write at a time**: the server runs writes one at a time across all connections. Concurrent writes to the same data never fail with a conflict; the later write wins
- **Commit**: just ends the transaction (the changes are already applied)
- **Rollback**: undoes created and deleted nodes and relationships, property and label changes, and index updates. The cancelling changes are written to the WAL too, so the rollback survives restarts and reaches followers
- **When the connection closes**: transactions that were neither committed nor rolled back are rolled back automatically
- **Read only**: in a transaction begun with `{"type": "begin", "readOnly": true}` (`client.transaction(read_only=True)` in Python), write queries fail
- **A query applies completely or not at all**: when a query fails midway (a constraint violation, for example), every change it made is undone. Inside a transaction, changes from earlier queries remain
- **On failure**: if the server crashes before a commit, the changes made so far remain after restart (they are not rolled back automatically)

Unless you need to be able to roll back, it is simpler to write the change as a single query without a transaction. One query can create and update several elements.

```cypher
MATCH (a:Account {id: "A"}), (b:Account {id: "B"})
SET a.balance = a.balance - 100, b.balance = b.balance + 100
```

## Writing large amounts of data

Instead of sending one query per item, expand a list parameter with `UNWIND` in a single query; it is faster.

```cypher
UNWIND $batch AS item
CREATE (n:Person {name: item.name, age: item.age})
```

Aim for about 1,000–10,000 items per request. See [Server configuration](../operations/server-config.md) for how fsyncs to the WAL are grouped.
