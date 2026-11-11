---
title: Home
nav_order: 1
description: "MaharitDB - High-performance graph database"
permalink: /
source_commit: afbcbdf0
---

# What is MaharitDB?

MaharitDB is a graph database engine written in Rust. It manages graph data made of nodes and edges efficiently, using a Cypher-like query language. It covers a wide range of scenarios, from embedded use to running as a network server.

## Key features

### Cypher-like query language

Work with graph data using syntax close to Neo4j's Cypher. Besides the basic operations such as `MATCH`, `CREATE`, `MERGE`, `SET`, `DELETE`, and `RETURN`, it supports advanced constructs such as `UNWIND`, `WITH`, `UNION`, `FOREACH`, and `CALL {}` subqueries.

```cypher
// Create nodes and connect them with a relationship
CREATE (alice:Person {name: "Alice", age: 30})

CREATE (bob:Person {name: "Bob", age: 25})

MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"})
CREATE (a)-[:KNOWS {since: 2020}]->(b)

// Query the graph
MATCH (p:Person)-[:KNOWS]->(friend:Person)
WHERE p.age > 20
RETURN p.name, friend.name
ORDER BY p.name
```

### BM25 full-text search (with Japanese support)

A built-in full-text index ranks results with the BM25 algorithm. It supports Japanese morphological analysis with lindera IPADIC, so Japanese text can be searched naturally. Phrase search and fuzzy search (with a configurable edit distance) are also supported.

```cypher
// Create a full-text index
CREATE FULLTEXT INDEX articleIndex FOR (a:Article) ON EACH [a.title, a.body]

// Search with BM25 scoring
CALL db.index.fulltext.search("articleIndex", "グラフデータベース")
YIELD node, score
RETURN node.title, score
ORDER BY score DESC
```

### Graph algorithms

The major algorithms from graph theory are built in.

- **Shortest path**: weighted and unweighted shortest paths with Dijkstra's algorithm and BFS
- **All-paths search**: enumerate every path within a given number of hops with depth-first search
- **PageRank**: importance ranking
- **Centrality**: betweenness centrality and closeness centrality
- **Connected components**: connected and strongly connected components with Union-Find
- **Community detection**: label propagation

### Persistence and the WAL

The server stores the whole graph in a single data file and appends each write query's changes to a write-ahead log (WAL) before replying. If the process is killed (`kill -9`, power loss, and so on), it replays the WAL at startup and restores every acknowledged write. Group commit combines the fsyncs of concurrent writes into one. The WAL is also used to stream replication. See [server configuration](operations/server-config.md) for details.

### TCP server with TLS/SSL

The built-in TCP server supports TLS 1.2/1.3 through rustls. Certificates can be hot-reloaded, so you can renew them without stopping the service.

### Prometheus metrics and structured logs

A monitoring HTTP server serves `/metrics` (Prometheus format) and `/health`. The server writes its activity as structured JSON logs, including the time each query took.

### Replication (leader/follower)

Leader/follower replication works by streaming the WAL. It monitors follower liveness with heartbeats and supports failover by promoting a follower with `maharit admin promote-to-leader`.

### Backup and restore

`maharit backup` and `maharit restore` back up and restore the data file (with gzip or zstd compression). The `maharit-storage` Rust API also offers incremental backups and point-in-time recovery (PITR) using the WAL.

## Use cases

- **Knowledge graphs**: model relationships between concepts naturally and run inference queries
- **Social graphs**: manage and analyze follow/follower relationships between users
- **Recommendation engines**: collaborative filtering powered by graph algorithms
- **Fraud detection**: find anomalous patterns in the graph structure
- **Dependency analysis**: manage dependency trees of packages or configurations
- **Maps and routing**: compute shortest paths over road networks

## How this documentation is organized

This documentation consists of the following sections.

| Section | Contents |
|----|------|
| Quick start | Go from installation to your first query in about 10 minutes |
| Cypher query language | A reference for every feature of the query syntax |
| Function reference | Built-in functions, aggregate functions, and full-text search functions |
| Indexes and constraints | Property indexes, full-text indexes, and schema constraints |
| Server and operations | Production configuration, TLS, authentication, monitoring, and backups |
| Advanced topics | Transactions, replication, and query optimization |
| API reference | The Rust client and the TCP protocol specification |
| Graph algorithms | How to use the built-in algorithms and their performance characteristics |
| Visualization | The web app, DOT/SVG export, and ASCII rendering |

!!! note "Translation status"
    The English documentation is being translated from the Japanese original. Pages that are not translated yet are shown in Japanese.

## License and contributing

MaharitDB is released under the MIT License. Bug reports, feature requests, and pull requests are welcome through the repository's Issues and Pull Requests.
