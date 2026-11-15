---
title: Access control (ACL)
parent: Server and operations
nav_order: 4
source_commit: ed074894
---

# Access control (ACL)

[Roles](auth.md) (`reader` / `writer` / `admin`) decide for the whole query whether a user may only read or may also write.
ACL rules go further: per label or per property, they keep a particular user or role from reading or writing.

- Nodes with a label the user cannot read are treated as **not existing** for that user (no error is raised)
- Properties the user cannot read are treated as absent (`null`)
- A statement that changes a node or property the user cannot write fails, and all of its changes are undone
- ACL rules never apply to the `admin` role (so an administrator cannot be locked out by mistake)

ACL rules only take effect on a server with authentication enabled (`--require-auth`). Without authentication everyone is treated as admin, so nothing is restricted.

## Defining rules

Only admin can define and list rules. Like the user-management statements, they are handled by the server (in the REPL or when embedding the engine they return an error).

### DENY

Forbids reading (`READ`) or writing (`WRITE`).

```cypher
// Hide :Secret nodes from the writer role
DENY READ ON :Secret TO ROLE writer
```

```cypher
// Hide the password property of :User from readers
DENY READ ON :User(password) TO ROLE reader
```

```cypher
// bob cannot change :Invoice nodes
DENY WRITE ON :Invoice TO USER bob
```

There are three kinds of target.

| Target | Meaning |
|---|---|
| `:Label` | Nodes with that label (and all of their properties) |
| `:Label(prop)` | The property `prop` of nodes with that label |
| `:*` | All nodes (including nodes without a label) |

The subject is `TO USER name` or `TO ROLE role` (`reader` / `writer` / `admin`, or their aliases). The user must exist.

### GRANT

Explicitly allows something, as an exception to a denial. Use it to let a particular user access something their role is denied.

```cypher
// Readers see nothing except :Public
DENY READ ON :* TO ROLE reader
```

```cypher
GRANT READ ON :Public TO ROLE reader
```

Both rules above are for the same role, so `DENY` wins and `:Public` stays hidden too (see "How rules are evaluated" below).
`GRANT` is meant for opening up a role's denial for individual users.

```cypher
// Readers cannot see :Report, except alice
DENY READ ON :Report TO ROLE reader
```

```cypher
GRANT READ ON :Report TO USER alice
```

ACL rules never extend a role's permissions. A `GRANT WRITE` for a `reader` user does not let it write.

### REVOKE

Removes the rules (both `GRANT` and `DENY`) for the same subject, target and action.

```cypher
REVOKE READ ON :Report FROM ROLE reader
```

A subject, target and action can only have one of `GRANT` or `DENY`. Setting one replaces the other.

### SHOW ACL

```cypher
SHOW ACL
```

Returns one row per rule with `subject` (`USER bob`, `ROLE read_only`, ...), `resource` (`:Secret`, `:User(password)`, `:*`), `action` (`READ` / `WRITE`) and `permission` (`GRANT` / `DENY`).

## How rules are evaluated

Whether a user may read or write a label (or a property) is decided from the rules that cover it, in this order.
A `:*` rule covers every label, and a `:Label` rule also covers all properties of that label.

1. If any rule names the user, those rules alone decide (denied if any of them is `DENY`, allowed if they are all `GRANT`)
2. Otherwise the rules for the user's role decide in the same way
3. If there are none, access is allowed

A node with several labels is hidden if any one of its labels cannot be read.

## What is covered

### Denied reads

- The node does not match `MATCH` patterns (and is not counted by `count(*)` or per-label counts)
- Relationships to hidden nodes are hidden as well
- Hidden nodes do not appear in `CALL db.labels()`, the algorithm procedures (`db.pageRank` and others) or full-text search (`db.index.fulltext.search`). Full-text search also leaves out nodes whose indexed properties include one the user cannot read
- An unreadable property is `null` for `n.prop` and is left out of `keys(n)` / `properties(n)`. `WHERE n.prop = …` and `{prop: …}` do not match it either
- Sessions with ACL rules do not narrow candidates with property indexes (an index would match on values the user cannot read). Results are the same, but queries can be slower

### Denied writes

If `CREATE`, `MERGE`, `SET`, `REMOVE`, `DELETE` or similar change a node with a label the user cannot write (including adding or removing labels), a property the user cannot write, or a relationship to a node the user cannot write, the whole statement fails with `access denied: …` and its changes are undone.

- Properties the user cannot read cannot be written either (so hidden values cannot be overwritten or removed)
- A node connected to hidden nodes cannot be deleted (not even with `DETACH DELETE`). As usual, `DELETE` without `DETACH` leaves a node with relationships in place, hidden ones included
- Creating and dropping indexes and constraints is not subject to ACL rules (only the role decides)

### Caveats

- Because unreadable nodes are invisible, `MERGE` finds no match and creates a new node. If a unique constraint exists, the constraint error reveals that a node exists
- Rules are per server. They are not sent to replication followers, so give followers the same `--users-file`
- With sharding, rules are enforced by the server of each shard

## Storage and audit log

Rules are saved in the `--users-file` file together with the users (the file's `version` is 3; a version 2 file from earlier releases is read with no rules and written as version 3 on the next save).
`DROP USER` also removes the rules that name that user. Without `--users-file` rules live only in memory and are lost on restart.

With `--audit-log`, adding or removing a rule is logged as `acl_changed`, and a write refused by ACL rules as `permission_denied` (`"action":"acl_write"`).
