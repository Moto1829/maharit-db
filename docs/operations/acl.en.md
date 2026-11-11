---
title: Access control (ACL)
parent: Server and operations
nav_order: 4
source_commit: ed074894
---

# Access control (ACL)

!!! warning "Not implemented"
    Access control per label or per property (`GRANT` / `REVOKE` / `DENY` and so on) is not available on the server yet.

The only access control on the current server is the three roles (`reader` / `writer` / `admin`) described in [Authentication and roles](auth.md). A role applies to the whole query: read only, read and write, or read and write plus user management. You cannot hide nodes with a particular label or particular properties.

## Workarounds for now

- Keep data that must not be visible in a separate database (a server with a different `--data`)
- Build queries in the application based on the user (limiting the columns and labels returned)
- Use users with the `reader` role for people who only need to read

## Next steps

The `maharit-server` crate has an `AclManager` (`auth.rs`) that evaluates per-label and per-property rules, but it is not connected to query execution. Enforcing it on the server and adding statements to define rules is future work.
