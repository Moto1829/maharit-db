---
title: TCP protocol
parent: API reference
nav_order: 3
source_commit: 44074b1d
---

# TCP protocol specification

This page describes the protocol between MaharitDB clients and the server. Refer to it when implementing your own client or to understand the communication at a low level.
The authoritative definitions are `Request` / `Response` in `crates/maharit-server/src/tcp_server.rs`.

## Overview

Length-prefixed JSON messages are exchanged over a TCP socket (optionally TLS). There is no handshake; requests can be sent right after connecting.

- The client sends one request and the server returns a response (only streaming queries return several responses)
- One connection can send any number of requests in sequence. Responses come back in the order the requests were sent

## Message format

Every message (in both directions) has this format.

```text
+---------------------+---------------------+
| Length (4 bytes)    | Payload (N bytes)   |
+---------------------+---------------------+
  uint32 big-endian     UTF-8 JSON
```

- **Length**: byte length of the payload (big-endian 32-bit unsigned integer). The maximum is 64 MiB (`MAX_MESSAGE_SIZE`); larger messages get an error
- **Payload**: a JSON object. The `type` field identifies the kind of message

Field names are case-sensitive. Optional fields may be omitted.

## Requests (client → server)

| `type` | Description | Fields |
|--------|------|-----------|
| `query` | Run a query and return all results at once | `query`, `params`?, `txId`?, `sessionToken`? |
| `streamQuery` | Run a query and return the results in chunks | `query`, `params`?, `chunkSize`? (default 100), `txId`?, `sessionToken`? |
| `login` | Log in and get a session token | `username`, `password` |
| `begin` | Begin a transaction | `readOnly`? (default `false`), `sessionToken`? |
| `commit` | Commit a transaction | `txId`, `sessionToken`? |
| `rollback` | Roll back a transaction | `txId`, `sessionToken`? |
| `ping` | Liveness check | none |
| `stats` | Get server statistics | none |
| `promote` | Promote this follower to leader (admin only; see [Replication](../advanced/replication.md)) | `sessionToken`? |
| `disconnect` | Close the connection | none |

`sessionToken` is required when the server runs with `--require-auth` (see [Authentication](../operations/auth.md)).

## Responses (server → client)

| `type` | In reply to | Fields |
|--------|-------------------|-----------|
| `result` | `query` | `rows` |
| `streamStart` / `streamChunk` / `streamEnd` | `streamQuery` | See below |
| `loggedIn` | `login` | `sessionToken`, `role`, `expiresAt` |
| `transactionBegun` | `begin` | `txId` |
| `committed` | `commit` | `txId` |
| `rolledBack` | `rollback` | `txId` |
| `pong` | `ping` | none |
| `stats` | `stats` | `connections`, `total_queries`, `nodes`, `edges`, `replication`? |
| `promoted` | `promote` | `replicationBind`? (the address where it started accepting followers) |
| `goodbye` | `disconnect`, or when the server stops | none |
| `error` | Any | `message` |
| `authError` | Any | `message` (missing, invalid, or expired token; insufficient permission; failed login) |

## Queries

### query

```json
{
  "type": "query",
  "query": "MATCH (n:Person {name: $name}) WHERE n.age >= $min_age RETURN n.name AS name, n.age AS age",
  "params": {"name": "Alice", "min_age": 25}
}
```

```json
{
  "type": "result",
  "rows": [
    {"name": "Alice", "age": 30}
  ]
}
```

- `rows` is an array of rows; each row is an object mapping column names to values
- Values keep their JSON types (string, number, boolean, `null`, array, object). Nodes, paths, dates, datetimes, and durations are returned as strings
- Write queries return counts (for example `{"created_nodes": 1, "created_edges": 0}`, `{"properties_set": 1}`)

### Query parameters (`params`)

`params` is an object of values for the `$name` placeholders in the query. The values are sent separately from the query string and are never interpreted as Cypher. **Pass user input in `params` instead of embedding it in the query string.**

| JSON | Cypher type |
|------|------------|
| `null` | null |
| `true` / `false` | Boolean |
| Integer (fits in i64) | Integer |
| Any other number | Float |
| String | String |
| Array | List |
| Object | Map |

If the query refers to a parameter that is not in `params`, the reply is an `error` (it is not treated as `null`). If `params` is omitted or empty, the query runs without parameters.
See [Parameters ($param syntax)](../cypher/parameters.md) for details on usage.

### streamQuery

```json
{"type": "streamQuery", "query": "MATCH (n:Person) RETURN n.name AS name", "chunkSize": 2}
```

The server sends responses in this order.

```json
{"type": "streamStart", "streamId": 0, "totalRows": 3}
{"type": "streamChunk", "streamId": 0, "chunkIndex": 0, "rows": [{"name": "Alice"}, {"name": "Bob"}]}
{"type": "streamChunk", "streamId": 0, "chunkIndex": 1, "rows": [{"name": "Charlie"}]}
{"type": "streamEnd", "streamId": 0, "totalRows": 3}
```

If the query fails, a single `error` is returned instead of `streamStart`.

## Authentication

```json
{"type": "login", "username": "alice", "password": "secret"}
```

```json
{"type": "loggedIn", "sessionToken": "3f9c…", "role": "read_write", "expiresAt": 1767225600}
```

- `role` is `admin` / `read_write` / `read_only`
- `expiresAt` is when the session expires (Unix epoch seconds)
- Add `"sessionToken": "3f9c…"` to later requests

## Transactions

```json
{"type": "begin"}
{"type": "query", "query": "CREATE (:Item {id: $id})", "params": {"id": 1}, "txId": 7}
{"type": "commit", "txId": 7}
```

Put the `txId` from the `begin` reply `{"type": "transactionBegun", "txId": 7}` on `query` and on `commit` / `rollback`.

## Errors

```json
{"type": "error", "message": "Parse error: …"}
```

`message` is a human-readable string with one of these prefixes.

| Prefix | Meaning |
|--------|------|
| `Invalid request:` | Not valid JSON, unknown `type`, or a required field is missing |
| `Parse error:` | Syntax error in the query |
| `Execution error:` | Runtime error (constraint violation, type error, undefined parameter, and so on) |

The connection remains usable after an error.

## Closing the connection

The client sends `{"type": "disconnect"}`. The server replies `{"type": "goodbye"}` and closes the connection. The server also sends `goodbye` when it stops.

## Example implementation

Python (standard library only):

```python
import json
import socket
import struct

def send(sock, msg):
    body = json.dumps(msg).encode("utf-8")
    sock.sendall(struct.pack(">I", len(body)) + body)

def recv(sock):
    def read_exact(n):
        buf = b""
        while len(buf) < n:
            chunk = sock.recv(n - len(buf))
            if not chunk:
                raise ConnectionError("connection closed")
            buf += chunk
        return buf
    (length,) = struct.unpack(">I", read_exact(4))
    return json.loads(read_exact(length))

with socket.create_connection(("127.0.0.1", 7687)) as sock:
    send(sock, {
        "type": "query",
        "query": "MATCH (n:Person {name: $name}) RETURN n.age AS age",
        "params": {"name": "Alice"},
    })
    print(recv(sock))  # {'type': 'result', 'rows': [{'age': 30}]}
```
