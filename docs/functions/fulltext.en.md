---
title: Full-text search functions
parent: Function reference
nav_order: 6
source_commit: f8dd9869
---

# Full-text search functions

MaharitDB has a built-in full-text search engine based on the BM25 algorithm. It also supports Japanese morphological analysis (lindera IPADIC).

## Creating a full-text index

Before using full-text search, create an index on the target nodes and properties.

```cypher
// Index on a single property
CREATE FULLTEXT INDEX articleIndex FOR (a:Article) ON EACH [a.title]

// Index on several properties
CREATE FULLTEXT INDEX articleIndex FOR (a:Article) ON EACH [a.title, a.body, a.summary]
```

Only one target label can be given (multiple labels such as `(n:A|B)` are not supported).

## CALL db.index.fulltext.search()

Runs a full-text search on an index.

```cypher
// Basic search
CALL db.index.fulltext.search("articleIndex", "Rust programming")
YIELD node, score
RETURN node.title, score
ORDER BY score DESC
LIMIT 10
```

Returned values:
- `node`: the matching node
- `score`: the BM25 score (higher is more relevant)

## Search query syntax

### Keyword search

Finds documents containing all of the keywords (AND search).

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

### Fuzzy search (edit distance)

Follow a term with a tilde (`~`) and an edit distance to tolerate spelling variations.

```cypher
// Allow variations within edit distance 1
CALL db.index.fulltext.search("articleIndex", "graph~1")
YIELD node, score
RETURN node.title, score

// Within edit distance 2
CALL db.index.fulltext.search("articleIndex", "datbase~2")
YIELD node, score
RETURN node.title, score
```

### OR search

Use the `OR` keyword to find documents containing any of the keywords.

```cypher
CALL db.index.fulltext.search("articleIndex", "Rust OR Go OR Python")
YIELD node, score
RETURN node.title, score
ORDER BY score DESC
```

## CONTAINS predicate

`CONTAINS` can also be used as a filter.

```cypher
MATCH (a:Article)
WHERE a.body CONTAINS "グラフ"
RETURN a.title
```

Note: `CONTAINS` is a substring match and does not produce a BM25 score. Use `CALL db.index.fulltext.search()` for ranked search.

## Japanese full-text search

MaharitDB supports Japanese morphological analysis with [lindera](https://github.com/lindera/lindera) and an embedded IPADIC dictionary. Japanese text is split into morphemes (words) automatically for indexing and search. This feature is for Japanese text.

```cypher
// Full-text search over Japanese text
CREATE FULLTEXT INDEX jpArticleIndex FOR (a:Article) ON EACH [a.title, a.body]

// Search by morphemes (「グラフデータベース」 is searched as 「グラフ」 and 「データベース」)
CALL db.index.fulltext.search("jpArticleIndex", "グラフデータベース")
YIELD node, score
RETURN node.title, score
ORDER BY score DESC

// Narrow down with several keywords
CALL db.index.fulltext.search("jpArticleIndex", "機械学習 グラフ")
YIELD node, score
RETURN node.title, score
ORDER BY score DESC
LIMIT 5
```

### Enabling the `japanese` feature

Japanese morphological analysis is **gated behind the `japanese` Cargo feature, which is off by default**. The embedded IPADIC dictionary greatly increases the binary size (lindera 6 and later embed it uncompressed), so enable it only in builds that need Japanese full-text search.

| Build | Binary size (measured) |
| --- | --- |
| Default (`japanese` off) | about 2.7MB |
| `japanese` on | about 49MB |

```bash
# Build a release binary with Japanese morphological analysis
cargo build --release -p maharit-server --features japanese

# Run the tests with the feature enabled as well
cargo test -p maharit-core --features japanese
```

At build time, lindera downloads the IPADIC source data (about 12MB) from lindera.dev and builds the dictionary (verified with MD5). Set `LINDERA_BUILD_DICTIONARY_CACHE_DIR` to a directory to cache the built dictionary; later builds then skip the download and work offline.

The `japanese` feature of `maharit-server` pulls in `maharit-core/japanese`, which enables the optional dependency `lindera` (with the `embed-ipadic` feature).

> **Behavior without the feature**: Japanese text is split on non-alphanumeric characters, like any other text. Japanese has no spaces between words, so a whole sentence tends to become one token and word-based search does not work in practice (substring matches with CONTAINS still work).

### Morphological analysis details

With the `japanese` feature on, the tokenizer does the following for each input text.

1. **Detecting Japanese**: if the text contains at least one hiragana (U+3040–U+309F), katakana (U+30A0–U+30FF), or CJK unified ideograph (U+4E00–U+9FFF) character, it is treated as Japanese and analyzed with lindera. Pure ASCII text without such characters uses the regular tokenizer (split on non-alphanumeric characters + lowercase) whether or not the feature is on.
2. **Dictionary**: the embedded IPADIC dictionary (`embedded://ipadic`) is loaded and text is segmented in `Mode::Normal`.
3. **Excluding stop parts of speech**: these parts of speech (IPADIC top-level categories) add noise to search and are dropped from the tokens:
   - 助詞 (particles) / 助動詞 (auxiliary verbs) / 記号 (symbols) / 接続詞 (conjunctions) / 感動詞 (interjections) / フィラー (fillers) / 非言語音 (non-verbal sounds)
4. **Normalizing to the base form**: each token is converted to its IPADIC base form (the uninflected form). If no base form is available or it is `*`, the surface form is used instead. Finally everything is lowercased.

Example: 「グラフは高速です」 → the particle 「は」 and the auxiliary verb 「です」 are dropped, leaving 「グラフ」 and 「高速」.

### Mixed Japanese and English text

If a text contains even one Japanese character, the whole string goes to the Japanese tokenizer (lindera). For mixed text such as 「Rust言語でグラフDBを実装」, how the English words are split depends on lindera's segmentation (they do not always remain as tokens). Only pure ASCII queries and documents go through the regular tokenizer.

### Fallback on errors

If loading the dictionary or the analysis fails for any reason, the tokenizer falls back to splitting on whitespace and non-alphanumeric characters, so the text to index is never silently dropped.

### Parallel index building

Bulk indexing of many nodes (`build_index` / `build_index_bulk`) tokenizes in parallel with rayon (from 200 documents up). lindera segmenters, including those used to tokenize search queries, are cached per thread, so the expensive dictionary load happens at most once per thread.

### Limitations

- The accuracy of the analysis depends on the IPADIC vocabulary. New words, proper nouns, and technical terms may be split unexpectedly (user dictionaries are not supported).
- Only the embedded IPADIC dictionary is available; switching to other dictionaries such as NEologd is not supported.
- Japanese is detected by character class, so some text, such as strings of only full-width alphanumerics, may not go to the Japanese tokenizer.

## BM25 scoring details

The BM25 implementation in MaharitDB uses this formula:

```
score(q, d) = Σ IDF(qi) * (f(qi,d) * (k1+1)) / (f(qi,d) + k1 * (1 - b + b * |d| / avgdl))
```

Parameters:
- `k1 = 1.2` (how much term frequency matters)
- `b = 0.75` (normalization by document length)
- IDF: `ln(1 + (N - df + 0.5) / (df + 0.5))` (a variant that avoids negative scores)

## Managing indexes

```cypher
// Drop an index
DROP FULLTEXT INDEX articleIndex

```

`SHOW INDEXES` lists property indexes only; full-text indexes are not included.

## Practical examples

```cypher
// Find related articles (filtered by a score threshold)
CALL db.index.fulltext.search("articleIndex", $query)
YIELD node, score
WHERE score > 0.5
RETURN node.title, node.url, score
ORDER BY score DESC
LIMIT 5

// Add more information to search results
CALL db.index.fulltext.search("articleIndex", "Rust async await")
YIELD node, score
MATCH (node)-[:HAS_TAG]->(tag:Tag)
RETURN node.title, score, collect(tag.name) AS tags
ORDER BY score DESC
```
