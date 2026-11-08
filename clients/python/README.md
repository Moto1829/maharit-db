# maharit — Python client for MaharitDB

A pure-Python client library for [MaharitDB](https://github.com/Moto1829/maharit-db), a graph database with Cypher query language support.

## Installation

```bash
pip install maharit
# With pandas support:
pip install maharit[pandas]
```

## Quick Start

### Synchronous API

```python
from maharit import Client

# Connect and use as context manager
with Client.connect("localhost:7687") as client:
    # Create nodes
    client.execute("CREATE (n:Person {name: 'Alice', age: 30})")
    client.execute("CREATE (n:Person {name: 'Bob', age: 25})")

    # Query nodes
    result = client.query("MATCH (n:Person) RETURN n.name, n.age")
    for row in result:
        print(f"{row['n.name']}: {row['n.age']}")

    # Convert to pandas DataFrame
    df = client.query("MATCH (n:Person) RETURN n.name, n.age").to_dataframe()
    print(df)
```

### Asynchronous API

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

### TLS

When the server runs with `--tls-cert` / `--tls-key`, pass the CA certificate
to trust (for a self-signed certificate, the certificate itself), or
`ssl=True` to verify against the system CA store. `AsyncClient` and both pools
take the same arguments.

```python
with Client.connect("db.example.com:7687", ca_file="ca.crt") as client:
    client.query("MATCH (n) RETURN count(n)")
```

### Transactions

```python
from maharit import Client

with Client.connect("localhost:7687") as client:
    # Context manager auto-commits or rolls back
    with client.transaction() as tx:
        tx.execute("CREATE (a:Person {name: 'Alice'})")
        tx.execute("CREATE (b:Person {name: 'Bob'})")
        tx.execute("MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) "
                   "CREATE (a)-[:KNOWS]->(b)")
```

### Streaming

```python
from maharit import Client

with Client.connect("localhost:7687") as client:
    for row in client.stream("MATCH (n) RETURN n", chunk_size=50):
        process(row)
```

## API Reference

### `Client`

| Method | Description |
|--------|-------------|
| `Client.connect(address, timeout=30.0, ssl=None, ca_file=None)` | Connect to the server (TLS with `ssl=True` or `ca_file=...`) |
| `client.execute(query)` | Execute a Cypher query |
| `client.query(query)` | Alias for `execute` |
| `client.stream(query, chunk_size=100)` | Stream results row by row |
| `client.begin_transaction(read_only=False)` | Begin a transaction |
| `client.transaction()` | Context manager for transactions |
| `client.ping()` | Check server health |
| `client.stats()` | Get server statistics |
| `client.close()` | Close the connection |

### `AsyncClient`

Same API as `Client` but all methods are `async` and streaming uses `async for`.

### `QueryResult`

| Method | Description |
|--------|-------------|
| `result[i]` | Get row by index |
| `len(result)` | Number of rows |
| `for row in result` | Iterate rows |
| `result.to_dataframe()` | Convert to pandas DataFrame |

## Protocol

The client communicates with MaharitDB using a simple length-prefixed JSON protocol:

- **Request**: `[4-byte big-endian length][JSON payload]`
- **Response**: `[4-byte big-endian length][JSON payload]`

## License

MIT
