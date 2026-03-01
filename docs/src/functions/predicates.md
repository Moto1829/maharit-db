# 述語関数

述語関数はリストや値に対してブール値を返す関数です。`WHERE` 句やリスト内包表記と組み合わせて使用します。

## リスト述語

### all(variable IN list WHERE condition)

リストのすべての要素が条件を満たす場合に `true` を返します。

```cypher
RETURN all(x IN [2, 4, 6, 8] WHERE x % 2 = 0)
-- 結果: true

RETURN all(x IN [1, 2, 3] WHERE x > 0)
-- 結果: true

RETURN all(x IN [1, 2, 3] WHERE x > 1)
-- 結果: false

-- パスのすべてのノードが条件を満たすか確認
MATCH p = (start:City)-[:ROAD*]->(end:City)
WHERE all(city IN nodes(p) WHERE city.accessible = true)
RETURN start.name, end.name
```

### any(variable IN list WHERE condition)

リストの少なくとも一つの要素が条件を満たす場合に `true` を返します。

```cypher
RETURN any(x IN [1, 2, 3] WHERE x > 2)
-- 結果: true

RETURN any(x IN [1, 2, 3] WHERE x > 5)
-- 結果: false

-- 友人の中に東京在住の人がいるか
MATCH (p:Person)-[:KNOWS]->(f:Person)
WITH p, collect(f) AS friends
WHERE any(friend IN friends WHERE friend.city = "Tokyo")
RETURN p.name
```

### none(variable IN list WHERE condition)

リストのどの要素も条件を満たさない場合に `true` を返します。`NOT any(...)` と等価です。

```cypher
RETURN none(x IN [1, 2, 3] WHERE x > 5)
-- 結果: true

RETURN none(x IN [1, 2, 3] WHERE x > 2)
-- 結果: false

-- ブロックされたユーザーがいないパスのみ
MATCH p = (a:Person)-[:KNOWS*]->(b:Person)
WHERE none(n IN nodes(p) WHERE n.blocked = true)
RETURN a.name, b.name
```

### single(variable IN list WHERE condition)

リストの中でちょうど一つの要素が条件を満たす場合に `true` を返します。

```cypher
RETURN single(x IN [1, 2, 3] WHERE x = 2)
-- 結果: true

RETURN single(x IN [1, 2, 3] WHERE x > 1)
-- 結果: false (2 つが条件を満たす)

-- 管理者が一人だけかを確認
MATCH (dept:Department {name: "Engineering"})
WITH [(dept)-[:HAS_MEMBER]->(m:Person) | m] AS members
WHERE single(m IN members WHERE m.role = "admin")
RETURN dept.name
```

## 存在確認

### exists(expression)

プロパティやパターンが存在するかを確認します。

```cypher
-- プロパティの存在確認
MATCH (n:Person)
WHERE exists(n.email)
RETURN n.name, n.email

-- パターンの存在確認
MATCH (n:Person)
WHERE exists((n)-[:FOLLOWS]->(:Person))
RETURN n.name

-- RETURN での使用
MATCH (n:Person)
RETURN n.name, exists(n.email) AS has_email
```

注意: プロパティの存在確認には `IS NOT NULL` も使えます。

```cypher
-- 以下は等価
WHERE exists(n.email)
WHERE n.email IS NOT NULL
```

### EXISTS {} サブクエリ（パターン存在確認）

より複雑なパターンの存在確認には `EXISTS {}` サブクエリを使用します。

```cypher
MATCH (p:Person)
WHERE EXISTS {
  MATCH (p)-[:KNOWS]->(f:Person)
  WHERE f.city = "Tokyo"
}
RETURN p.name
```

## 空チェック

### isEmpty(value)

リストまたは文字列が空かどうかを確認します。

```cypher
-- リストが空かどうか
RETURN isEmpty([])
-- 結果: true

RETURN isEmpty([1, 2, 3])
-- 結果: false

-- 文字列が空かどうか
RETURN isEmpty("")
-- 結果: true

RETURN isEmpty("hello")
-- 結果: false

-- プロパティのリストが空でないノードを検索
MATCH (n:Article)
WHERE NOT isEmpty(n.tags)
RETURN n.title, n.tags
```

## 実用的な例

```cypher
-- すべての必須フィールドが設定されているユーザー
MATCH (u:User)
WHERE all(field IN ["name", "email", "created_at"] WHERE exists(u[field]))
RETURN u

-- 少なくとも 1 つのスキルが条件を満たすエンジニア
MATCH (e:Engineer)
WHERE any(skill IN e.skills WHERE skill IN ["Rust", "Go", "C++"])
RETURN e.name, e.skills

-- 同一パス上に同じ人が現れない（シンプルパス確認）
MATCH p = (a:Person)-[:KNOWS*1..5]->(b:Person)
WHERE a <> b
  AND single(n IN nodes(p) WHERE n = a)
RETURN a.name, b.name, length(p) AS hops
```
