---
title: FOREACH / サブクエリ
parent: Cypher クエリ言語
nav_order: 5
---

# FOREACH / サブクエリ

## FOREACH

`FOREACH` はリストの各要素に対して更新操作（`CREATE`、`SET`、`MERGE`、`DELETE`、`REMOVE`）を繰り返し実行します。`RETURN` は使用できません。

### 基本的な使い方

```cypher
-- リストの各要素でノードを作成
FOREACH (name IN ["Alice", "Bob", "Charlie"] |
  CREATE (:Person {name: name})
)
```

### パスのすべてのノードを更新

```cypher
-- パス上のすべてのノードにラベルを追加
MATCH p = (start:Person {name: "Alice"})-[:KNOWS*]->(end:Person)
FOREACH (n IN nodes(p) |
  SET n.visited = true
)
```

### MATCH と組み合わせた使用

```cypher
-- タグリストを展開してリレーションを作成
MATCH (a:Article {id: $id})
FOREACH (tag IN $tags |
  MERGE (t:Tag {name: tag})
  CREATE (a)-[:TAGGED_WITH]->(t)
)
```

### ネストした FOREACH

```cypher
-- 二重リストを処理
FOREACH (row IN [[1, "Alice"], [2, "Bob"]] |
  FOREACH (item IN row |
    -- 内側の処理（実際の用途は限定的）
    CREATE (:Log {value: item})
  )
)
```

### 条件付き更新（CASE との組み合わせ）

`FOREACH` は条件分岐の代わりとしても使用できます。条件が真の場合のみ実行したい場合：

```cypher
-- 条件が真の場合のみ実行（CASE でリストの長さを 0 または 1 に制御）
MATCH (n:Person)
FOREACH (_ IN CASE WHEN n.age >= 18 THEN [1] ELSE [] END |
  SET n:Adult
)
```

## CALL {} サブクエリ

`CALL {}` ブロック内にサブクエリを記述できます。サブクエリは外部クエリのバインディングを参照できます（インポート変数）。

### 基本的な使い方

```cypher
-- 外部クエリとは独立したサブクエリ
CALL {
  MATCH (n:Person)
  RETURN count(n) AS person_count
}
RETURN person_count
```

### 外部変数のインポート

```cypher
-- 外部クエリの変数を参照
MATCH (p:Person)
CALL {
  WITH p
  MATCH (p)-[:FRIENDS]->(f:Person)
  RETURN count(f) AS friend_count
}
RETURN p.name, friend_count
```

### サブクエリによる集計

```cypher
-- 各人の友人数を集計
MATCH (p:Person)
CALL {
  WITH p
  MATCH (p)-[:KNOWS]->(friend:Person)
  RETURN count(friend) AS num_friends,
         collect(friend.name) AS friend_names
}
RETURN p.name, num_friends, friend_names
ORDER BY num_friends DESC
```

### EXISTS {} サブクエリ

パターンの存在確認に使用します（`WHERE` 句内）。

```cypher
-- フォロワーが存在する人のみ返す
MATCH (p:Person)
WHERE EXISTS {
  MATCH (p)<-[:FOLLOWS]-(:Person)
}
RETURN p.name
```

### COUNT {} サブクエリ

サブクエリの結果件数を数えます。

```cypher
-- フォロワー数を取得
MATCH (p:Person)
RETURN p.name,
       COUNT { MATCH (p)<-[:FOLLOWS]-(:Person) } AS follower_count
ORDER BY follower_count DESC
```

### COLLECT {} サブクエリ

サブクエリの結果をリストとして収集します。

```cypher
-- 各人の友人名をリストで取得
MATCH (p:Person)
RETURN p.name,
       COLLECT { MATCH (p)-[:KNOWS]->(f:Person) RETURN f.name } AS friends
```

### ユニオンサブクエリ

`CALL {}` 内で `UNION` を使用できます。

```cypher
MATCH (p:Person {name: "Alice"})
CALL {
  WITH p
  MATCH (p)-[:FOLLOWS]->(f:Person)
  RETURN f.name AS name, "follows" AS rel_type
  UNION
  WITH p
  MATCH (p)-[:KNOWS]->(f:Person)
  RETURN f.name AS name, "knows" AS rel_type
}
RETURN name, rel_type
```
