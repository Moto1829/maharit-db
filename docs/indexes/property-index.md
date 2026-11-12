---
title: プロパティインデックス
parent: インデックス・制約
nav_order: 1
---

# プロパティインデックス

プロパティインデックスを作成すると、特定のプロパティによる検索が、ラベルの全ノードの走査からインデックスの検索（等価条件はハッシュ、範囲条件は B-tree）になり、速くなります。

## インデックスの作成

```cypher
// ラベルとプロパティにインデックスを作成
CREATE INDEX ON :Person(name)

CREATE INDEX ON :User(email)
```

構文: `CREATE INDEX ON :Label(property)`。Neo4j 4.x 以降と同じ `CREATE INDEX FOR (n:Label) ON (n.property)` の形でも書けます（インデックス名と複数プロパティの指定には対応していません）。

```cypher
CREATE INDEX FOR (n:Person) ON (n.email)
```

> **注意**: 既存ノードにインデックスを作成した場合、作成時点で対象ラベルを持つ全ノードが自動的にインデックス登録されます。インデックス作成後のノードの追加・削除や、`SET` / `REMOVE` によるプロパティ・ラベルの変更も自動的に反映されます。

## インデックスの確認

```cypher
SHOW INDEXES
```

出力例:

```
label   property
------  --------
Person  name
User    email
```

## インデックスの削除

```cypher
DROP INDEX ON :Person(name)
```

構文: `DROP INDEX ON :Label(property)`

## インデックスが使用される条件

インライン・プロパティ指定（`{prop: value}` 形式）でインデックスが自動的に使用されます。

```cypher
// インデックスが使用される（最もパフォーマンスが高い）
MATCH (n:Person {name: "Alice"}) RETURN n

// 範囲検索（IntまたはFloat型プロパティのインデックスで高速化）
MATCH (n:Person) WHERE n.age > 30 RETURN n

MATCH (n:Person) WHERE n.age >= 25 AND n.age <= 40 RETURN n
```

## EXPLAIN でインデックス使用を確認

インデックスが使用されているかどうかは `EXPLAIN` で確認できます。

```cypher
EXPLAIN MATCH (n:Person {name: "Alice"}) RETURN n.name
```

インデックスが使用されている場合の出力例:

```
Operator                    Est. Rows     Cost Details
------------------------------------------------------------------------
IndexSeek                           1        2 :Person.name
Projection                          1        1 columns: n.name
```

インデックスが未作成の場合は `NodeByLabelScan` が表示されます:

```
Operator                    Est. Rows     Cost Details
------------------------------------------------------------------------
NodeByLabelScan                    11        2 :Person
Projection                         11        1 columns: n.name
```

WHERE 句の条件でインデックスが適用される場合は `IndexRangeScan` が表示されます:

```
Operator                    Est. Rows     Cost Details
------------------------------------------------------------------------
IndexRangeScan                      2        1 :Person.age
Projection                          2        1
```

## インデックス設計のベストプラクティス

- **選択性が高いプロパティ**: 値の種類が多いプロパティ（名前、ID など）にインデックスを作成する
- **頻繁に検索するプロパティ**: `WHERE` 句またはインライン指定で頻繁に使用するプロパティを優先する
- **インデックス過多に注意**: インデックスが多すぎるとノード作成・更新パフォーマンスが低下する

## 現在の制限事項

- 1つのインデックスは 1 ラベル × 1 プロパティのみ（複合インデックスは未対応）
- エッジ（リレーションシップ）のプロパティインデックスは未対応
- インデックスの判定はノードの最初のラベルで行う（複数ラベルを持つノードでは、最初のラベルのインデックスだけが使われる）
