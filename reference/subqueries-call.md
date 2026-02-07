# CALL subqueries

Source: https://neo4j.com/docs/cypher-manual/current/subqueries/call-subquery/

## 概要
- `CALL { ... }` は行ごとに実行されるサブクエリ。
- 内部での更新（`CREATE`/`MERGE`/`SET`/`DELETE`）が可能。

## 変数スコープ
- 外側変数は**スコープ句**で明示的にインポート：`CALL (a, b)` / `CALL (*)` / `CALL ()`。
- スコープ句なしは非推奨。
- インポート変数は再宣言不可、別名付け不可。
- 外側と同名の変数を返す場合はリネームが必要。

## 実行順序
- 入力行の順序は未定義。必要なら `CALL` 直前に `ORDER BY`。

## Optional/Conditional
- `OPTIONAL CALL` は行が見つからない場合 `null` を返す。
- `WHEN ... THEN ... ELSE ...` による条件分岐（Cypher 25 / Neo4j 2025.06）。

## 返却/単位サブクエリ
- `RETURN` ありは**返却サブクエリ**（行数が変化）。
- `RETURN` なしは**単位サブクエリ**（行数維持、更新用）。

## UNIONとの併用
- `CALL` 内で `UNION` 可能。集計や結果再構成に有効。
