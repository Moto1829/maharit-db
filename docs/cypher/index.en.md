---
title: Cypher query language
nav_order: 4
has_children: true
source_commit: 340d3428
---

# Cypher query language

MaharitDB supports the Neo4j-compatible Cypher query language.

## Standards conformance

The tests run the expression scenarios of the openCypher TCK (a collection of scenarios that check the meaning of the language), as of 2026-11-21:

| Area | Passing / scenarios |
|---|---|
| null | 44 / 44 |
| Conditional expressions (CASE) | 13 / 13 |
| Arithmetic | 6 / 6 |
| Boolean | 141 / 150 |
| Comparison | 61 / 72 |
| Lists | 144 / 185 |
| Literals | 80 / 131 |
| Precedence | 46 / 105 |
| Strings | 15 / 32 |
| Temporal | 20 / 1004 |

Most temporal scenarios fail because `localdatetime` / `time` / `localtime` and dates given by week or ordinal day are not supported.

