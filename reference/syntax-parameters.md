# Parameters

Source: https://neo4j.com/docs/cypher-manual/current/syntax/parameters/

## 概要
- パラメータ化により文字列連結を避け、実行計画キャッシュの再利用性が向上。

## 使用可能な箇所
- リテラル/式/ID/動的プロパティ参照。
- 動的ラベル/関係タイプの指定も可能（別節参照）。

## 使用できない構造
- プロパティキー、関係タイプ、ラベルを**直接**置き換える構文は不可。
  - 例: `MATCH (n:$param)` や `MATCH (n)-[:$param]->(m)` は不可。

## パラメータ名の規則
- 先頭/途中に `_`・文字・数字・結合文字（アクセント等）を利用可能。
- 通貨記号・制御文字・空白・句読点は不可。

## 自動パラメータ化（Neo4j 5+）
- クエリ内リテラルを自動的にパラメータ化する挙動がある。
- 意図的なパラメータ指定が推奨。

## 設定方法（例）
- Cypher Shell/Browser: `:param name => 'Joe'`
- Drivers/HTTP APIはクライアント仕様に依存。

## 参考
- 動的プロパティのフィルタ: https://neo4j.com/docs/cypher-manual/current/clauses/where/#filter-on-dynamic-properties
- 動的ラベル/型: https://neo4j.com/docs/cypher-manual/current/clauses/match/#dynamic-match
