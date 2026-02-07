# Indexes

Source: https://neo4j.com/docs/cypher-manual/current/indexes/

## 概要
- インデックスは、一次データ（ノード/関係/プロパティ）のコピーを保持して検索効率を向上させる。
- 作成後は自動的に構築・更新される。

## インデックスの種類
### Search-performance indexes（厳密一致向け）
- Range（デフォルト）
- Text
- Point
- Token lookup

### Semantic indexes（近似/類似検索向け）
- Full-text
- Vector

## 参照リンク
- Search-performance indexes: https://neo4j.com/docs/cypher-manual/current/indexes/search-performance-indexes/
- Semantic indexes: https://neo4j.com/docs/cypher-manual/current/indexes/semantic-indexes/
