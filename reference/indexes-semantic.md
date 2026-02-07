# Semantic indexes

Source: https://neo4j.com/docs/cypher-manual/current/indexes/semantic-indexes/

## 概要
- Semantic indexes は意味的/類似度に基づく検索を提供。
- 近似スコアを返す。
- Search-performance indexes と異なり自動利用されない。
  - Full-text: 手続きで明示的に呼び出し。
  - Vector: `SEARCH` 句、または手続き。

## 種類
- Full-text indexes: `STRING` 内容の意味検索と類似度。
- Vector indexes: ベクトル埋め込みの類似検索。

## 参照リンク
- Full-text indexes: https://neo4j.com/docs/cypher-manual/current/indexes/semantic-indexes/full-text-indexes/
- Vector indexes: https://neo4j.com/docs/cypher-manual/current/indexes/semantic-indexes/vector-indexes/
