---
title: Indexes and constraints
nav_order: 6
has_children: true
source_commit: eefb1a37
---

# Indexes and constraints

How to set up property indexes, full-text indexes, and schema constraints.

## Persistence and replication

`maharit server` stores the definitions of indexes, constraints, and full-text indexes in the data file and the WAL. After a restart (including recovery from a crash) the definitions come back, and the index contents are rebuilt from the data at startup. On a large graph, startup takes correspondingly longer.

With replication, the definitions are also sent to followers (see [Replication](../advanced/replication.md)).
