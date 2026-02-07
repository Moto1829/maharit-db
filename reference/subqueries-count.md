# COUNT subqueries

Source: https://neo4j.com/docs/cypher-manual/current/subqueries/count/

## 概要
- サブクエリの行数を数える。

## 特徴
- 外側変数は**インポート不要**で参照可能。
- `RETURN` は省略可能（`UNION DISTINCT` の場合は必須）。
- `MATCH` がパターンのみの場合は省略可能。

## ルール
- 非書き込みクエリのみ。
- `COUNT` は式として `RETURN`/`SET`/`CASE` などで使用可能。
- `UNION` 併用可。

## Conditional
- `WHEN ... THEN ... ELSE ...` による条件分岐（Cypher 25）。
