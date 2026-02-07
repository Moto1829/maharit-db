# Aggregating functions

Source: https://neo4j.com/docs/cypher-manual/current/functions/aggregating/

## 概要
- 集計関数は行集合を単一値に集約。
- グルーピングキーで集約単位が決まる。

## 主要関数
- `avg`, `collect`, `count`, `min`, `max`, `percentileCont`, `percentileDisc`, `stDev`, `stDevP`, `sum`

## 重要ルール
- `count(*)` は `null` 行もカウント、`count(expr)` は `null` を除外。
- `collect()` は `null` を除外。
- `ALL`/`DISTINCT` で重複制御（`ALL` はGQL整合）。

## 集計式の制約
- 集計式内の部分式は「集計関数 / 定数 / パラメータ / グルーピングキー / ローカル変数」のみ許可。
- 複雑なキーは `WITH` で事前投影。
