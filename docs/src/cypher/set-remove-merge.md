# SET / REMOVE / MERGE

## SET

`SET` はノードやエッジのプロパティを設定・更新します。

### 基本的な使い方

```cypher
-- プロパティを設定
MATCH (n:Person {name: "Alice"})
SET n.age = 31

-- 複数プロパティを同時に設定
MATCH (n:Person {name: "Alice"})
SET n.age = 31, n.email = "alice@example.com"

-- エッジのプロパティを設定
MATCH (a:Person)-[r:KNOWS]->(b:Person)
WHERE a.name = "Alice" AND b.name = "Bob"
SET r.since = 2022
```

### ラベルを追加する

`SET n:Label` でノードにラベルを追加します。既存のラベルはそのまま保持されます。

```cypher
-- ラベルを追加（既存ラベルは保持）
MATCH (n:Person {name: "Alice"})
SET n:Admin
-- Alice は Person と Admin の両方のラベルを持つ

-- 一度に複数のラベルを追加
MATCH (n:Person {name: "Alice"})
SET n:Admin:Moderator
-- Alice は Person・Admin・Moderator の3つのラベルを持つ
```

追加後のラベル一覧は `labels(n)` で確認できます。

```cypher
MATCH (n:Person {name: "Alice"})
SET n:Admin
RETURN n.name, labels(n)
-- 結果: ["Person", "Admin"]
```

### マップでプロパティを一括設定

```cypher
-- マップを使った一括更新（既存プロパティは維持）
MATCH (n:Person {name: "Alice"})
SET n += {age: 32, city: "Tokyo"}

-- マップで全プロパティを置き換え（既存プロパティは削除）
MATCH (n:Person {name: "Alice"})
SET n = {name: "Alice", age: 32}
```

### MATCH+SET の組み合わせ

```cypher
-- フィルタして更新
MATCH (n:Person)
WHERE n.age < 18
SET n:Minor

-- 計算式で更新
MATCH (n:Product)
SET n.price = n.price * 1.1
```

## REMOVE

`REMOVE` はプロパティやラベルを削除します。

### プロパティを削除する

```cypher
-- 特定のプロパティを削除
MATCH (n:Person {name: "Alice"})
REMOVE n.temporary_flag

-- 複数プロパティを同時に削除
MATCH (n:Person {name: "Alice"})
REMOVE n.flag1, n.flag2
```

### ラベルを削除する

`REMOVE n:Label` で特定のラベルを削除します。他のラベルは保持されます。

```cypher
-- ラベルを削除（他のラベルは保持）
MATCH (n:Person:Temp {name: "Alice"})
REMOVE n:Temp
-- Alice は Person のみになる

-- 複数のラベルを同時に削除
MATCH (n)
WHERE n:Draft AND n:Review
REMOVE n:Draft, n:Review
SET n:Published
```

## MERGE

`MERGE` はパターンが存在しない場合のみ作成します。既存であればそのまま使用します。「UPSERT」に近い動作です。

### 基本的な使い方

```cypher
-- 存在しなければ作成
MERGE (n:Person {name: "Alice"})

-- 存在確認後に作成・更新を分岐
MERGE (n:Person {name: "Alice"})
ON CREATE SET n.created_at = 2024, n.age = 30
ON MATCH SET n.updated_at = 2024
```

### エッジの MERGE

```cypher
-- リレーションシップが存在しなければ作成
MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"})
MERGE (a)-[:KNOWS]->(b)

-- ON CREATE / ON MATCH を使用
MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"})
MERGE (a)-[r:KNOWS]->(b)
ON CREATE SET r.since = 2024
ON MATCH SET r.last_seen = 2024
```

### パターン全体の MERGE

```cypher
-- ノードとエッジを含むパターン全体をマージ
MERGE (a:Person {name: "Alice"})-[:WORKS_AT]->(c:Company {name: "Acme"})
```

注意: パターン全体が一致するかどうかを確認します。パターンの一部のみが存在する場合も、存在しないとみなして全体を作成します。複雑なパターンでは、ノードとエッジを個別に `MERGE` することを推奨します。

### MERGE の典型的なユースケース

```cypher
-- ユーザーのログイン記録（初回はノードを作成、以降は更新）
MERGE (u:User {email: $email})
ON CREATE SET
  u.created_at = $now,
  u.login_count = 1
ON MATCH SET
  u.last_login = $now,
  u.login_count = u.login_count + 1
RETURN u
```

## SET / REMOVE / MERGE の選択指針

| 操作 | 使用する構文 |
|------|------------|
| プロパティを追加・更新 | `SET` |
| プロパティを削除 | `REMOVE` |
| ラベルを追加 | `SET n:Label` |
| ラベルを削除 | `REMOVE n:Label` |
| 存在しなければ作成（UPSERT） | `MERGE` |
| 作成時と更新時で異なる処理 | `MERGE ... ON CREATE ... ON MATCH` |
