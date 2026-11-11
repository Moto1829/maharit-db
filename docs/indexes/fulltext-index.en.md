---
title: Full-text indexes
parent: Indexes and constraints
nav_order: 2
source_commit: 52f38528
---

# Full-text indexes

Full-text indexes enable ranked search over text data, with BM25 scoring and Japanese morphological analysis.

## Creating an index

```cypher
// Create a basic full-text index
CREATE FULLTEXT INDEX articleIndex FOR (a:Article) ON EACH [a.title]

// Index several properties
CREATE FULLTEXT INDEX articleIndex FOR (a:Article) ON EACH [a.title, a.body, a.summary]
```

Only one target label can be given (multiple labels as in Neo4j's `(n:A|B)` are not supported).

### Syntax

```
CREATE FULLTEXT INDEX <index_name>
FOR (<variable>:<Label>)
ON EACH [<variable>.<property>[, <variable>.<property>...]]
```

`ON (<variable>.<property>, ...)` can be used instead of `ON EACH [...]`.

## Dropping an index

```cypher
DROP FULLTEXT INDEX articleIndex
```

## Listing indexes

`SHOW INDEXES` lists property indexes only; full-text indexes are not included. There is no statement to list full-text indexes yet.

## Running a full-text search

Search an index with `CALL db.index.fulltext.search()`.

```cypher
CALL db.index.fulltext.search("articleIndex", "Rust プログラミング")
YIELD node, score
RETURN node.title, score
ORDER BY score DESC
LIMIT 10
```

### Parameters

| Parameter | Type | Description |
|------------|-----|------|
| `indexName` | String | Name of the index to search |
| `query` | String | Search query |

### Returned columns

| Column | Type | Description |
|------|-----|------|
| `node` | Node | The matching node |
| `score` | Float | BM25 score |

## Writing search queries

### Single keyword

```cypher
CALL db.index.fulltext.search("articleIndex", "Rust")
YIELD node, score
RETURN node.title, score
```

### Several keywords (AND search)

Separate keywords with spaces. Documents containing all of the keywords rank highest.

```cypher
CALL db.index.fulltext.search("articleIndex", "グラフ データベース")
YIELD node, score
RETURN node.title, score
ORDER BY score DESC
```

### Phrase search

Enclose a phrase in double quotes to search for it exactly.

```cypher
CALL db.index.fulltext.search("articleIndex", "\"グラフデータベース\"")
YIELD node, score
RETURN node.title, score
```

### Fuzzy search

Give an edit distance with `~N` to tolerate spelling variations.

```cypher
CALL db.index.fulltext.search("articleIndex", "databse~1")
YIELD node, score
RETURN node.title, score
```

## Japanese support

Full-text search in MaharitDB supports Japanese morphological analysis with lindera (IPADIC dictionary), when built with the `japanese` feature (see [Full-text search functions](../functions/fulltext.md)). This feature is for Japanese text.

```cypher
// Index Japanese text
CREATE FULLTEXT INDEX jpIndex FOR (d:Document) ON EACH [d.content]

// Search through morphological analysis
// 「機械学習」 is split into 「機械」 and 「学習」 for the search
CALL db.index.fulltext.search("jpIndex", "機械学習")
YIELD node, score
RETURN node.content, score
ORDER BY score DESC
```

## Combining with more data

Further query the full-text search results.

```cypher
CALL db.index.fulltext.search("articleIndex", "Rust async")
YIELD node AS article, score
WHERE score > 1.0
MATCH (article)-[:AUTHORED_BY]->(author:Person)
MATCH (article)-[:HAS_TAG]->(tag:Tag)
RETURN article.title, author.name, collect(tag.name) AS tags, score
ORDER BY score DESC
LIMIT 5
```

## Performance notes

- Full-text indexes are updated automatically as nodes are created, changed, and deleted
- With many documents, building an index can take time
- A score threshold (`WHERE score > N`) filters out unneeded results
- The more properties an index covers, the larger it gets
