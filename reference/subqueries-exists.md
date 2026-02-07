# EXISTS subqueries

Source: https://neo4j.com/docs/cypher-manual/current/subqueries/existential/

## 概要
- パターンが1件以上存在するかを判定。
- `MATCH` を内包し、パスパターン式より強力。

## 特徴
- 外側変数は**インポート不要**で参照可能。
- `RETURN` は省略可能（戻り値は外側に伝播しない）。
- `MATCH` がパターンのみの場合は省略可能。

## ルール
- 非書き込みクエリのみ。
- `EXISTS` が1行でも返せば `true`。
- `UNION` 併用可（`RETURN` 有無は分岐で統一）。

## Conditional
- `WHEN ... THEN ... ELSE ...` による条件分岐（Cypher 25）。
