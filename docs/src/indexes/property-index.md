# プロパティインデックス

プロパティインデックスを作成することで、特定のプロパティに対するクエリを高速化できます。インデックスのないプロパティの検索は全ノードスキャンになります。

## インデックスの作成

```cypher
-- ラベルとプロパティにインデックスを作成
CREATE INDEX FOR (n:Person) ON (n.name)

-- 複合インデックス（複数プロパティ）
CREATE INDEX FOR (n:Person) ON (n.city, n.age)

-- インデックスに名前を付ける
CREATE INDEX personNameIndex FOR (n:Person) ON (n.name)
```

## インデックスの確認

```cypher
-- すべてのインデックスを表示
SHOW INDEXES
```

出力例:

```
+------------------+--------+-----------+------------+
| name             | label  | property  | state      |
+------------------+--------+-----------+------------+
| personNameIndex  | Person | name      | ONLINE     |
| index_person_age | Person | age       | ONLINE     |
+------------------+--------+-----------+------------+
```

## インデックスの削除

```cypher
-- 名前でインデックスを削除
DROP INDEX personNameIndex

-- ラベル・プロパティ指定で削除
DROP INDEX FOR (n:Person) ON (n.name)
```

## インデックスが使用される条件

次のようなクエリ条件でインデックスが使用されます。

```cypher
-- 等値比較（最もインデックスが効果的）
MATCH (n:Person {name: "Alice"}) RETURN n
MATCH (n:Person) WHERE n.name = "Alice" RETURN n

-- 範囲比較
MATCH (n:Person) WHERE n.age > 30 RETURN n
MATCH (n:Person) WHERE n.age >= 25 AND n.age <= 40 RETURN n

-- 前方一致
MATCH (n:Person) WHERE n.name STARTS WITH "Al" RETURN n
```

インデックスが使用されないケース:
- `ENDS WITH`、`CONTAINS`（プロパティインデックスでは不可）
- 計算式（`WHERE n.age * 2 > 60` のような場合）

## EXPLAIN でインデックス使用を確認

```cypher
EXPLAIN MATCH (n:Person {name: "Alice"}) RETURN n
```

出力にインデックススキャンが表示されることを確認します：

```
NodeIndexSeek[n:Person(name)] → Filter → Return
```

インデックスが使われていない場合は `NodeLabelScan` や `AllNodesScan` が表示されます。

## インデックス設計のベストプラクティス

- **選択性が高いプロパティ**: 値の種類が多いプロパティ（名前、ID など）にインデックスを作成する
- **頻繁に検索するプロパティ**: `WHERE` 句で頻繁に使用するプロパティを優先する
- **複合インデックス**: 常にセットで検索するプロパティは複合インデックスにまとめる
- **インデックス過多に注意**: インデックスが多すぎると書き込みパフォーマンスが低下する

## リレーションシップのインデックス

```cypher
-- エッジのプロパティにインデックスを作成
CREATE INDEX FOR ()-[r:KNOWS]-() ON (r.since)

-- 削除
DROP INDEX FOR ()-[r:KNOWS]-() ON (r.since)
```
