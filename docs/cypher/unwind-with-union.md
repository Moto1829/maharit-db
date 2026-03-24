---
title: UNWIND / WITH / UNION
parent: Cypher クエリ言語
nav_order: 4
---

# UNWIND / WITH / UNION

## UNWIND

`UNWIND` はリストを展開し、各要素を個別の行として扱います。リストを入力として受け取り、各要素ごとに以降の処理を実行します。

### 基本的な使い方

```cypher
-- リストリテラルを展開
UNWIND [1, 2, 3] AS num
RETURN num

-- 結果:
-- num: 1
-- num: 2
-- num: 3
```

### リストプロパティの展開

```cypher
-- ノードのタグリストを展開
MATCH (a:Article)
UNWIND a.tags AS tag
RETURN a.title, tag
```

### データのバルクインサート

```cypher
-- パラメータで渡したリストからノードを作成
UNWIND $persons AS person
CREATE (n:Person {name: person.name, age: person.age})
```

パラメータの例（JSON）:
```json
{
  "persons": [
    {"name": "Alice", "age": 30},
    {"name": "Bob", "age": 25},
    {"name": "Charlie", "age": 35}
  ]
}
```

### UNWIND と MATCH の組み合わせ

```cypher
-- ID リストからノードを検索
UNWIND [1, 2, 3] AS id
MATCH (n:Product)
WHERE n.id = id
RETURN n.name, n.price
```

### ネストしたリストの展開

```cypher
-- 二重リストを展開
UNWIND [[1, 2], [3, 4], [5, 6]] AS sublist
UNWIND sublist AS num
RETURN num
```

## WITH

`WITH` はクエリを複数のステージに分け、中間結果を次のステージに渡します。集計結果を使ったフィルタリングや、クエリの論理的な区切りに使用します。

### 中間結果のフィルタ

```cypher
-- 友人が 3 人以上いる人を検索
MATCH (p:Person)-[:FRIENDS]->(f:Person)
WITH p, count(f) AS friend_count
WHERE friend_count >= 3
RETURN p.name, friend_count
ORDER BY friend_count DESC
```

### 集計後のさらなる処理

```cypher
-- 平均年齢より年上の人を検索
MATCH (p:Person)
WITH avg(p.age) AS avg_age
MATCH (p:Person)
WHERE p.age > avg_age
RETURN p.name, p.age, avg_age
```

### パイプライン処理

```cypher
-- ステージ1: 条件でフィルタ
MATCH (p:Person)-[:WORKS_AT]->(c:Company)
WHERE c.industry = "Tech"
WITH p, c

-- ステージ2: さらに条件を追加
MATCH (p)-[:KNOWS]->(colleague:Person)
WITH p, c, collect(colleague.name) AS colleagues

-- ステージ3: 結果を返す
RETURN p.name, c.name AS company, colleagues
ORDER BY p.name
LIMIT 10
```

### 変数のスコープ

`WITH` に含めなかった変数は後続のステージで使用できなくなります。

```cypher
MATCH (p:Person), (c:Company)
WHERE p.employer = c.name
WITH p  -- c はここで破棄
RETURN p.name
-- ここでは c は参照できない
```

## UNION

`UNION` は複数のクエリ結果を結合します。デフォルトでは重複を排除します。

### 基本的な使い方

```cypher
-- 二つのクエリ結果を結合（重複排除）
MATCH (n:Person) RETURN n.name AS name
UNION
MATCH (n:Company) RETURN n.name AS name
```

### UNION ALL（重複を保持）

```cypher
-- 重複を保持して結合
MATCH (n:Person) RETURN n.name AS name, "person" AS type
UNION ALL
MATCH (n:Company) RETURN n.name AS name, "company" AS type
```

### 複数のクエリを結合

```cypher
-- 異なる条件のクエリを結合
MATCH (n:Person) WHERE n.city = "Tokyo" RETURN n.name AS name, n.city AS location
UNION
MATCH (n:Person) WHERE n.city = "Osaka" RETURN n.name AS name, n.city AS location
UNION
MATCH (n:Person) WHERE n.city = "Kyoto" RETURN n.name AS name, n.city AS location
```

注意: `UNION` で結合する各クエリは同じ列数・列名を持つ必要があります。

### UNION と ORDER BY

`ORDER BY` は `UNION` の後（最後のクエリの後）に記述します。

```cypher
MATCH (n:Person) RETURN n.name AS name
UNION
MATCH (n:Company) RETURN n.name AS name
ORDER BY name ASC
LIMIT 20
```
