---
title: Authentication and roles
parent: Server and operations
nav_order: 3
source_commit: ed074894
---

# Authentication and roles

The MaharitDB server supports login with a user name and password, and access control (RBAC) with three roles.

## Enabling authentication

Authentication is **off** by default. With it off, the server accepts every request without login (and logs a warning at startup). Always enable it when the server is exposed on a network.

```bash
maharit server --host 0.0.0.0 --port 7687 \
  --require-auth --admin-password "$MAHARIT_ADMIN_PASSWORD"
```

- `--require-auth`: require a session token on every request and check role permissions
- `--admin-password <PW>` (env `MAHARIT_ADMIN_PASSWORD`): password of the `admin` user created at startup. Required with `--require-auth`; without it the server does not start

!!! warning "Users are lost on restart"
    Users and passwords live only in memory; they are not saved in the data file or in backups. After a restart only `admin` remains, so recreate users made with `CREATE USER` after each start.

## Login and sessions

Clients log in with a `login` request and put the returned session token in the `sessionToken` of later requests (see [Protocol](../api/protocol.md)).

```json
{"type": "login", "username": "alice", "password": "s3cret-pass"}
```

```json
{"type": "loggedIn", "sessionToken": "9f1c…", "role": "read_only", "expiresAt": 1767225600}
```

- Tokens are 256-bit random values
- A session expires after **30 minutes** without requests (each use extends it). Once it expires, requests get `authError`; log in again
- There is no logout request. Closing the connection leaves the session until it expires

With the Rust client, call `login` and the token is attached to later queries automatically.

```rust
use maharit_client::Client;

let mut client = Client::connect("127.0.0.1:7687").await?;
client.login("alice", "s3cret-pass").await?;
let result = client.query("MATCH (n:Person) RETURN n.name").await?;
```

The Python client has no login API yet.

## Roles

| Role | Alias | Allowed |
|---|---|---|
| `reader` | `read_only` | Read queries (`MATCH … RETURN`, `SHOW CONSTRAINTS` / `SHOW INDEXES`, procedure calls, and so on) |
| `writer` | `read_write` | Reads, plus writes (`CREATE` / `MERGE` / `SET` / `DELETE` / `REMOVE`, and so on) and creating and dropping indexes and constraints |
| `admin` | | Everything. User management (`CREATE USER` and so on, including `SHOW USERS`) is admin only |

Whether a query writes is decided by the kind of statement. Without permission, the reply is `authError` (`permission denied: …`).

There is no finer-grained control by label or property yet (see [Access control (ACL)](acl.md)).

## User management

Run user-management statements on a server with authentication enabled, logged in as admin. Roles can be written as strings or identifiers.

### CREATE USER

```cypher
CREATE USER alice SET PASSWORD 's3cret-pass' SET ROLE reader
```

The `SET` before `ROLE` is optional (`CREATE USER bob SET PASSWORD 'pw' ROLE writer`). A role is required.

### ALTER USER

```cypher
ALTER USER alice SET PASSWORD 'new-pass'
```

```cypher
ALTER USER alice SET ROLE writer
```

```cypher
ALTER USER alice SET PASSWORD 'new-pass' SET ROLE admin
```

### DROP USER

```cypher
DROP USER alice
```

### SHOW USERS

```cypher
SHOW USERS
```

Each row has `username`, `role` (`admin` / `read_write` / `read_only`), and `active`.

```text
username | role       | active
---------+------------+-------
admin    | admin      | true
alice    | read_only  | true
```

## How passwords are stored

Passwords are hashed with a per-user salt and PBKDF2-HMAC-SHA256 (100,000 iterations); plaintext is never kept. There are no rules on password length or character classes.

## Audit log

An audit log of logins and user-management operations is not built into the server yet.
