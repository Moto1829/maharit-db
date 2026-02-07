# Search-performance indexes

Source: https://neo4j.com/docs/cypher-manual/current/indexes/search-performance-indexes/

## 概要
- Search-performance indexes は厳密一致/範囲検索などの高速化を目的としたインデックス群。
- Cypher プランナが `MATCH` の開始点を決定する際に自動的に利用する。
- 利用可能な種類:
  - Range: 既定の汎用インデックス。ほとんどの述語に対応。
  - Text: `STRING` 述語向け。`CONTAINS`/`ENDS WITH` に最適。
  - Point: `POINT` 述語向け。距離やバウンディングボックスに最適。
  - Token lookup: ノードのラベル/関係タイプのみを解決。DB 作成時に既定で存在。

## 使い分け
- Range: 等価・範囲・前方一致などの汎用検索に適用。
- Text: `CONTAINS`/`ENDS WITH` を含む文字列検索を最適化。誤字近似は不可。
- Point: 空間検索（距離/範囲内）。
- Token lookup: ラベル/タイプ条件のみに利用。

## 自動利用とヒント
- プランナは複数の候補から最適なインデックスを選ぶ。
- 強制する場合は `USING` 句のヒントを用いる（詳細はインデックス・ヒント参照）。

## 参照リンク
- Create indexes: https://neo4j.com/docs/cypher-manual/current/indexes/search-performance-indexes/create-indexes/
- Show indexes: https://neo4j.com/docs/cypher-manual/current/indexes/search-performance-indexes/list-indexes/
- Drop indexes: https://neo4j.com/docs/cypher-manual/current/indexes/search-performance-indexes/drop-indexes/
- The impact of indexes on query performance: https://neo4j.com/docs/cypher-manual/current/indexes/search-performance-indexes/using-indexes/
- Index hints for the Cypher planner: https://neo4j.com/docs/cypher-manual/current/indexes/search-performance-indexes/index-hints/
