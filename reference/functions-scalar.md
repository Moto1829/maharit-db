# Scalar functions

Source: https://neo4j.com/docs/cypher-manual/current/functions/scalar/

## 概要
- 単一値を返す関数群。

## 主要関数
- 文字列長: `char_length`, `character_length`, `size`
- `coalesce`, `nullIf`
- ID系: `elementId`（推奨）, `id`（非推奨）
- パス/関係: `length`, `startNode`, `endNode`, `type`
- プロパティ: `properties`, `keys`
- 変換: `toBoolean/Float/Integer` + `OrNull`
- その他: `randomUUID`, `timestamp`, `valueType`

## 重要ルール
- `elementId()` はトランザクション外で安定しない。
- `size()` は LIST/STRING/VECTOR に対応。
