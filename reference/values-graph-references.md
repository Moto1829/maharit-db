# Graph references

Source: https://neo4j.com/docs/cypher-manual/current/values-and-types/graph-references/

## 概要
- `USE` 句や管理コマンドで対象グラフを指定するための値。

## 静的参照
- データベース/エイリアス名を直接指定。
- 複合DBでは `<composite>.<alias>` を使用。

## 動的参照
- `graph.byName(<string>)`
- `graph.byElementId(<elementId>)`
- `graph.propertiesByName(<string>)`

## ルール
- 識別子指定は**シンボリック名のエスケープ**規則。
- `graph.byName()` は**文字列リテラルのエスケープ**規則。
- 特殊文字を含む名前はバッククォートまたは文字列で指定。
