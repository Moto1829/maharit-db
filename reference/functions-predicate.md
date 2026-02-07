# Predicate functions

Source: https://neo4j.com/docs/cypher-manual/current/functions/predicate/

## 概要
- `BOOLEAN` を返す関数群。`WHERE` のフィルタに使用。

## 関数
- `all`, `any`, `none`, `single`
- `exists`（パターン存在判定）
- `isEmpty`（`STRING`/`MAP`/`LIST` が空か）
- `allReduce`（Cypher 25 / 2025.08）

## 重要ルール
- 空リストの挙動: `all`/`none` は `true`、`any`/`single` は `false`。
- `null` 入力や `null` 評価は `null` を返す。
- `exists()` は `EXISTS` サブクエリより簡易。
