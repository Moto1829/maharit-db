# ORDER BY / LIMIT / SKIP

## ORDER BY

`ORDER BY` で結果をソートします。`RETURN` の後に記述します。

### 基本的な使い方

```cypher
-- 名前の昇順（デフォルト）
MATCH (n:Person)
RETURN n.name, n.age
ORDER BY n.name

-- 年齢の降順
MATCH (n:Person)
RETURN n.name, n.age
ORDER BY n.age DESC

-- 昇順を明示
MATCH (n:Person)
RETURN n.name, n.age
ORDER BY n.age ASC
```

### 複数キーでのソート

```cypher
-- 都市名の昇順、同じ都市内では年齢の降順
MATCH (n:Person)
RETURN n.name, n.city, n.age
ORDER BY n.city ASC, n.age DESC
```

### エイリアスでのソート

```cypher
-- RETURN で定義したエイリアスでソート可能
MATCH (n:Person)-[:KNOWS]->(f:Person)
RETURN n.name AS name, count(f) AS friend_count
ORDER BY friend_count DESC, name ASC
```

### null 値の扱い

ソート時、`null` 値は昇順では最後、降順では最初に配置されます。

```cypher
MATCH (n:Person)
RETURN n.name, n.score
ORDER BY n.score DESC
-- score が null のノードは最初に来る
```

## LIMIT

`LIMIT` は返す行数の上限を指定します。

```cypher
-- 最初の 10 件を返す
MATCH (n:Person)
RETURN n.name, n.age
LIMIT 10

-- ORDER BY と組み合わせて上位を取得
MATCH (n:Person)
RETURN n.name, n.age
ORDER BY n.age DESC
LIMIT 5
```

### パラメータとしての LIMIT

```cypher
MATCH (n:Person)
RETURN n.name
LIMIT $limit
```

## SKIP

`SKIP` は指定した行数をスキップしてから結果を返します。ページネーションに使用します。

```cypher
-- 最初の 10 件をスキップして次の 10 件を返す（2 ページ目）
MATCH (n:Person)
RETURN n.name, n.age
ORDER BY n.name
SKIP 10
LIMIT 10
```

### ページネーションのパターン

```cypher
-- ページ番号とページサイズをパラメータで受け取る
MATCH (n:Person)
RETURN n.name, n.age
ORDER BY n.name
SKIP $page_size * ($page - 1)
LIMIT $page_size
```

パラメータの例：
```json
{
  "page": 2,
  "page_size": 10
}
```

## 節の順序

`ORDER BY`、`SKIP`、`LIMIT` の記述順序は次の通りです。

```
RETURN ...
ORDER BY ...
SKIP ...
LIMIT ...
```

```cypher
-- すべてを組み合わせた例
MATCH (n:Person)
WHERE n.age >= 18
RETURN n.name, n.age, n.city
ORDER BY n.age DESC, n.name ASC
SKIP 20
LIMIT 10
```

## WITH での使用

`WITH` の後にも `ORDER BY`、`SKIP`、`LIMIT` を使用できます。

```cypher
-- 上位 10 人を取得してさらに処理
MATCH (p:Person)-[:KNOWS]->(f:Person)
WITH p, count(f) AS friends
ORDER BY friends DESC
LIMIT 10
MATCH (p)-[:WORKS_AT]->(c:Company)
RETURN p.name, friends, c.name AS company
```

## パフォーマンスの注意

- `LIMIT` はできるだけ早い段階で適用することで処理を効率化できます
- インデックスがある場合、`ORDER BY` と `LIMIT` を組み合わせることで早期終了が可能になることがあります
- `EXPLAIN` を使ってクエリプランを確認してください
