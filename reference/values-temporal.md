# Temporal values

Source: https://neo4j.com/docs/cypher-manual/current/values-and-types/temporal/

## 型
- `DATE`, `LOCAL TIME`, `ZONED TIME`, `LOCAL DATETIME`, `ZONED DATETIME`（インスタント）
- `DURATION`（時間量、負値可）

## タイムゾーン
- UTCオフセット/名前（IANA）をサポート。
- 内部保存はUTC、表示でオフセット適用。
- `db.temporal.timezone` が既定タイムゾーンに影響。

## 形式
- `date`, `time`, `localdatetime`, `datetime` などの関数で生成/解析。
- `T` で日付と時刻を結合。

## Duration
- `P[nY][nM][nW][nD][T[nH][nM][nS]]` 形式。
- 月/日/秒グループで成分が管理される。
- 日/月/年の長さは一定ではない点に注意。

## インデックス/比較
- すべての時間型はインデックス可能。
- 範囲検索はインスタント型のみ。

## 参照
- 関連関数: https://neo4j.com/docs/cypher-manual/current/functions/temporal/
- 比較/順序: https://neo4j.com/docs/cypher-manual/current/values-and-types/ordering-equality-comparison/
