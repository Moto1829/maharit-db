# Lists

Source: https://neo4j.com/docs/cypher-manual/current/values-and-types/lists/

## リテラル
- `[...]` で作成。
- 異種要素/`null` を含められる。

## インデックス/スライス
- 0始まり。
- `list[i]` / `list[i..j]`（終端は排他的）。
- 範囲外の単一要素は `null`、範囲外スライスは切り詰め。

## サイズ
- `size(list)`。

## プロパティ保存
- **同種の単純型**リストのみ保存可能。
- `VECTOR` を含むリストは保存不可。

## 参照
- `IN` 演算子: https://neo4j.com/docs/cypher-manual/current/expressions/predicates/list-operators/
