# Cypher Overview

Source: https://neo4j.com/docs/cypher-manual/current/introduction/cypher-overview/

## 概要
- CypherはNeo4jの宣言的グラフクエリ言語。SQLと同様に「何を取得するか」に焦点を当て、取得方法はエンジンに委ねる。
- 2011年にNeo4jエンジニアにより設計。プロパティグラフDB向けのSQL相当として位置づけられる。
- ASCIIアート風のパターン表記を採用し、`(nodes)-[:CONNECT_TO]->(otherNodes)` のようにノードと関係を視覚的に表現する。

## CypherとSQLの主な違い
- **スキーマ柔軟性**
	- Neo4j/Cypherはスキーマに柔軟。ノード/リレーションシップは同じラベルや型でも必須プロパティを持つ必要はない。
	- ただしインデックス/制約による部分的スキーマ強制は推奨される（例: プロパティ存在制約）。
- **クエリの並び順**
	- SQLは取得項目を冒頭に書く一方、Cypherは`RETURN`が末尾になるのが基本構造。
	- 例: `MATCH ... WHERE ... RETURN ...`
- **クエリの簡潔性**
	- ノードと関係をパターンとして表現できるため、JOINの多いSQLに比べて短く直感的になりやすい。

## APOCとの関係
- Neo4jはAPOC (Awesome Procedures on Cypher) Coreライブラリをサポート。
- APOCはデータ統合、グラフアルゴリズム、データ変換などCypherの拡張領域を提供する。
- 仕様詳細はAPOCドキュメント参照: https://neo4j.com/docs/apoc/current/
