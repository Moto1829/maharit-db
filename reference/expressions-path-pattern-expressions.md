# Path pattern expressions

Source: https://neo4j.com/docs/cypher-manual/current/expressions/predicates/path-pattern-expressions/

## 概要
- `EXISTS` サブクエリに近いが、より簡潔なパス存在判定。

## 制約
- **少なくとも1つの関係**を含むパスパターンが必須。
- **新しい変数の宣言は不可**（既存変数のみ参照）。
- **グラフパターンの部分的な意味論**のみ利用可能。
- 述語が期待される位置（`WHERE` 等）でのみ使用可能。

## 注意
- `(:Person)` のような単一ノードはパスパターン式ではなく、単なるラベル述語。
- 述語式として使う場合は `NOT` や `exists()` 等で `BOOLEAN` 化が必要。

## 参考
- EXISTSサブクエリ: https://neo4j.com/docs/cypher-manual/current/subqueries/existential/
