# WHERE 句・フィルタリング

`WHERE` 句を使用して、`MATCH` で取得したノードやエッジをフィルタリングできます。

## 基本的な使い方

```cypher
MATCH (n:Person)
WHERE n.age > 25
RETURN n.name, n.age
```

`WHERE` は `MATCH` の直後に記述します。複数の `MATCH` 句がある場合は、最後の `MATCH` の後に一度まとめて書くことが多いです。

## 比較演算子

| 演算子 | 意味 | 例 |
|--------|------|----|
| `=` | 等しい | `n.age = 30` |
| `<>` | 等しくない | `n.age <> 30` |
| `<` | より小さい | `n.age < 30` |
| `<=` | 以下 | `n.age <= 30` |
| `>` | より大きい | `n.age > 30` |
| `>=` | 以上 | `n.age >= 30` |

```cypher
-- 年齢が 25 以上 40 以下の人
MATCH (n:Person)
WHERE n.age >= 25 AND n.age <= 40
RETURN n.name

-- 特定の名前以外
MATCH (n:Person)
WHERE n.name <> "Alice"
RETURN n.name
```

## 論理演算子

| 演算子 | 意味 |
|--------|------|
| `AND` | 論理積 |
| `OR` | 論理和 |
| `NOT` | 否定 |
| `XOR` | 排他的論理和 |

```cypher
-- AND の使用
MATCH (n:Person)
WHERE n.age > 20 AND n.city = "Tokyo"
RETURN n.name

-- OR の使用
MATCH (n:Person)
WHERE n.city = "Tokyo" OR n.city = "Osaka"
RETURN n.name

-- NOT の使用
MATCH (n:Person)
WHERE NOT n.name = "Bob"
RETURN n.name

-- 複合条件
MATCH (n:Person)
WHERE (n.age > 30 OR n.role = "admin") AND NOT n.deleted = true
RETURN n.name
```

## 文字列述語

```cypher
-- 前方一致
MATCH (n:Person)
WHERE n.name STARTS WITH "Al"
RETURN n.name

-- 後方一致
MATCH (n:Person)
WHERE n.email ENDS WITH "@example.com"
RETURN n.name

-- 部分一致
MATCH (n:Person)
WHERE n.bio CONTAINS "engineer"
RETURN n.name
```

## IN 演算子

リスト内のいずれかの値と一致するかを確認します。

```cypher
MATCH (n:Person)
WHERE n.city IN ["Tokyo", "Osaka", "Kyoto"]
RETURN n.name, n.city
```

## IS NULL / IS NOT NULL

プロパティの存在チェックを行います。

```cypher
-- email が設定されていないユーザー
MATCH (n:Person)
WHERE n.email IS NULL
RETURN n.name

-- email が設定されているユーザー
MATCH (n:Person)
WHERE n.email IS NOT NULL
RETURN n.name, n.email
```

## パターン条件

`WHERE` 句でパターンの存在確認ができます。

```cypher
-- フォロワーが存在するユーザー
MATCH (n:Person)
WHERE (n)-[:FOLLOWS]->(:Person)
RETURN n.name

-- 特定のラベルを持つノードと接続しているもの
MATCH (n:Person)
WHERE NOT (n)-[:BLOCKED]->(:Person)
RETURN n.name
```

## ラベルチェック

```cypher
-- 特定のラベルを持つかどうか確認
MATCH (n)
WHERE n:Person OR n:Company
RETURN n.name
```

## 範囲指定

```cypher
-- プロパティが範囲内にあることを確認
MATCH (p:Product)
WHERE 100 <= p.price <= 1000
RETURN p.name, p.price
```

## プロパティパターン内フィルタとの使い分け

プロパティが等値条件の場合、パターン内に直接記述することも `WHERE` 句で記述することも等価です。

```cypher
-- パターン内フィルタ（簡潔）
MATCH (n:Person {name: "Alice"}) RETURN n

-- WHERE 句（可読性が高い場合もある）
MATCH (n:Person)
WHERE n.name = "Alice"
RETURN n
```

ただし、等値以外の条件（`>`, `<`, `STARTS WITH` など）は必ず `WHERE` 句を使用します。
