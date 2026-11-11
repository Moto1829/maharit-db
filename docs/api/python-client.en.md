---
title: Python client
parent: API reference
nav_order: 2
source_commit: 80a2d7b0
---

# Python client

The maharit Python client library is a pure-Python implementation for connecting to MaharitDB and running Cypher queries. The client's own [README](https://github.com/Moto1829/maharit-db/tree/main/clients/python) covers the same ground.

## Installation

It is not published on PyPI yet; install it from the Git repository.

```bash
pip install "git+https://github.com/Moto1829/maharit-db#subdirectory=clients/python"
# With pandas support:
pip install "maharit[pandas] @ git+https://github.com/Moto1829/maharit-db#subdirectory=clients/python"
```

There is no login API yet (for connecting to a server with `--require-auth`).

## Synchronous API

The `Client` class is a socket-based synchronous client.

```python
from maharit import Client

with Client.connect("localhost:7687") as client:
    # Create nodes
    client.execute("CREATE (n:Person {name: 'Alice', age: 30})")
    client.execute("CREATE (n:Person {name: 'Bob', age: 25})")

    # Run a query
    result = client.query("MATCH (n:Person) RETURN n.name, n.age")
    for row in result:
        print(f"{row['n.name']}: {row['n.age']}")

    # Convert to a pandas DataFrame
    df = client.query("MATCH (n:Person) RETURN n.name, n.age").to_dataframe()
    print(df)
```

### Queries with parameters

Do not embed values such as user input in the query string (do not build it with f-strings or `%`). Refer to them with `$name` and pass them in `params`. The values are sent separately from the query and are never interpreted as Cypher.

```python
with Client.connect("localhost:7687") as client:
    client.execute(
        "CREATE (n:Person {name: $name, age: $age})",
        {"name": user_input, "age": 30},
    )
    result = client.query(
        "MATCH (n:Person) WHERE n.name IN $names RETURN n.name, n.age",
        {"names": ["Alice", "Bob"]},
    )

    # query_with_params(query, params) does the same
    result = client.query_with_params(
        "MATCH (n:Person {name: $name}) RETURN n.age", {"name": "Alice"}
    )
```

!!! warning "`query_with_params` in 0.2 and earlier"
    The old `query_with_params` did not send parameters to the server; it embedded the values into the query string without escaping (a value containing quotes could rewrite the query). Update both the server and the client.

### Connecting over TLS

If the server runs with `--tls-cert` / `--tls-key`, give `ca_file` (the trusted CA certificate, or the certificate itself when self-signed) or `ssl=True` (verify with the system CA store). `AsyncClient`, `ConnectionPool`, and `AsyncConnectionPool` take the same arguments (see [TLS/SSL](../operations/tls.md)).

```python
with Client.connect("db.example.com:7687", ca_file="/etc/maharit/ca.crt") as client:
    client.query("MATCH (n) RETURN count(n)")
```

## Async API

The `AsyncClient` class is an asyncio-based asynchronous client.

```python
import asyncio
from maharit import AsyncClient

async def main():
    async with AsyncClient.connect("localhost:7687") as client:
        await client.execute("CREATE (n:Person {name: 'Charlie'})")

        result = await client.query("MATCH (n:Person) RETURN n.name")
        for row in result:
            print(row["n.name"])

        # Stream large results
        async for row in client.stream("MATCH (n) RETURN n"):
            print(row)

asyncio.run(main())
```

## Transactions

```python
from maharit import Client

with Client.connect("localhost:7687") as client:
    # The context manager commits or rolls back automatically
    with client.transaction() as tx:
        tx.execute("CREATE (a:Person {name: 'Alice'})")
        tx.execute("CREATE (b:Person {name: 'Bob'})")
        tx.execute(
            "MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) "
            "CREATE (a)-[:KNOWS]->(b)"
        )
    # Rolled back automatically if an exception is raised
```

Async transactions:

```python
async with AsyncClient.connect("localhost:7687") as client:
    async with client.transaction() as tx:
        await tx.execute("CREATE (n:Item {id: 1})")
        await tx.execute("CREATE (n:Item {id: 2})")
```

## Streaming

Use the streaming API for large amounts of data.

```python
from maharit import Client

with Client.connect("localhost:7687") as client:
    # Process one row at a time (memory efficient)
    for row in client.stream("MATCH (n) RETURN n", chunk_size=50):
        process(row)
```

## pandas DataFrames

Convert query results directly to a pandas DataFrame.

```python
import pandas as pd
from maharit import Client

with Client.connect("localhost:7687") as client:
    df = client.query(
        "MATCH (n:Person) RETURN n.name AS name, n.age AS age"
    ).to_dataframe()

    print(df.head())
    print(df.dtypes)

    # Use everything pandas offers
    print(df[df["age"] > 25])
    print(df.groupby("name").mean())
```

## Connection pool

When several threads or tasks use the server, use a connection pool.

```python
from maharit import ConnectionPool

pool = ConnectionPool("localhost:7687", max_size=10)
with pool.connection() as client:
    client.query("MATCH (n) RETURN count(n)")
pool.close_all()
```

The async version is `AsyncConnectionPool` (`async with pool.connection() as client:`).

## API reference

### `Client`

| Method | Description |
|--------|------|
| `Client.connect(address, timeout=30.0, ssl=None, ca_file=None)` | Connect to a server (`ssl` / `ca_file` for TLS) |
| `client.execute(query, params=None)` | Run a Cypher query (`params` is a dict of values for `$name`) |
| `client.query(query, params=None)` | Alias of `execute` |
| `client.query_with_params(query, params)` | Same as `execute(query, params)` |
| `client.stream(query, chunk_size=100, params=None)` | Stream results one row at a time |
| `client.begin_transaction(read_only=False)` | Begin a transaction |
| `client.transaction()` | Context manager for transactions |
| `client.ping()` | Check that the server responds |
| `client.stats()` | Get server statistics |
| `client.close()` | Close the connection |

### `AsyncClient`

The same API as `Client`, with every method `async`.
Use `async for` for streaming.

### `QueryResult`

| Method | Description |
|--------|------|
| `result[i]` | Get a row by index |
| `len(result)` | Number of rows |
| `for row in result` | Iterate over rows |
| `result.to_dataframe()` | Convert to a pandas DataFrame |

### `Transaction` / `AsyncTransaction`

| Method | Description |
|--------|------|
| `tx.execute(query, params=None)` | Run a query in the transaction |
| `tx.commit()` | Commit |
| `tx.rollback()` | Roll back |

## Exceptions

| Exception | Description |
|------|------|
| `MaharitError` | Base exception class |
| `ConnectionError` | Connection error |
| `QueryError` | Query error |
| `TransactionError` | Transaction error |

## Protocol

The client communicates with a 4-byte big-endian length prefix followed by a JSON payload.

```
[4-byte big-endian u32 length][JSON payload]
```

It uses only the Python standard library and has no external dependencies (pandas is optional).
