# COLLECT subqueries

Source: https://neo4j.com/docs/cypher-manual/current/subqueries/collect/

## 概要
- サブクエリの戻り行をリストとして収集。

## 仕様
- `RETURN` 必須、**1列のみ**返却。
- 外側変数は**インポート不要**で参照可能。
- `UNION` 併用可。

## ルール
- 非書き込みクエリのみ。
- 変数シャドーイングは禁止。
- `collect()` と異なり `null` を自動除外しない。

## Conditional
- `WHEN ... THEN ... ELSE ...` による条件分岐（Cypher 25）。
