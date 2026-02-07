# Constraints

Source: https://neo4j.com/docs/cypher-manual/current/constraints/

## 概要
- データ品質/整合性を保証するための制約を提供。

## 制約種別
- Property uniqueness constraints: ノード/関係タイプ単位でプロパティ組合せの一意性。
- Property existence constraints: 対象ラベル/タイプの全要素にプロパティ存在（Enterprise）。
- Property type constraints: 対象プロパティ型の強制（Enterprise）。
- Key constraints: プロパティ存在＋組合せ一意性（Enterprise）。

## 参照リンク
- Create/show/drop constraints: https://neo4j.com/docs/cypher-manual/current/constraints/managing-constraints/
- Constraints syntax: https://neo4j.com/docs/cypher-manual/current/constraints/syntax/
