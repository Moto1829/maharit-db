---
title: スキーマ制約
parent: インデックス・制約
nav_order: 3
---

# スキーマ制約

スキーマ制約を定義することで、データの整合性を維持できます。制約に違反する操作はエラーとなります。

## CREATE CONSTRAINT

### UNIQUE 制約

特定のラベルのノードで、指定プロパティの値が一意であることを保証します。

```cypher
-- 単一プロパティの UNIQUE 制約
CREATE CONSTRAINT ON (n:Person) ASSERT n.email IS UNIQUE

-- 複合 UNIQUE 制約（複数プロパティの組み合わせが一意）
CREATE CONSTRAINT ON (n:Product) ASSERT (n.sku, n.warehouse) IS UNIQUE
```

UNIQUE 制約が存在する場合、重複する値を持つノードを作成しようとするとエラーになります：

```cypher
CREATE (n:Person {email: "alice@example.com"})
-- 2 回目の実行はエラー: Unique constraint violation on Person.email
```

### NOT NULL 制約

特定のプロパティが `null` になれないことを保証します。

```cypher
CREATE CONSTRAINT ON (n:Person) ASSERT n.name IS NOT NULL
CREATE CONSTRAINT ON (n:Product) ASSERT n.price IS NOT NULL
```

### 型制約

プロパティの値が特定の型であることを保証します。

```cypher
-- 整数型
CREATE CONSTRAINT ON (n:Person) ASSERT n.age IS INTEGER

-- 文字列型
CREATE CONSTRAINT ON (n:Person) ASSERT n.name IS STRING

-- 真偽値型
CREATE CONSTRAINT ON (n:Setting) ASSERT n.enabled IS BOOLEAN

-- 浮動小数点型
CREATE CONSTRAINT ON (n:Product) ASSERT n.price IS FLOAT
```

サポートされる型: `INTEGER`、`FLOAT`、`STRING`、`BOOLEAN`

### ラベル制約

```cypher
-- ノードが特定のラベルを持つことを保証（将来のバージョンで対応予定）
CREATE CONSTRAINT ON (n:Employee) ASSERT n:Person
```

## DROP CONSTRAINT

制約を削除します。

```cypher
-- 名前で制約を削除
DROP CONSTRAINT personEmailUnique

-- 条件を指定して削除
DROP CONSTRAINT ON (n:Person) ASSERT n.email IS UNIQUE
```

## SHOW CONSTRAINTS

現在定義されているすべての制約を表示します。

```cypher
SHOW CONSTRAINTS
```

出力例:

```
+------------------------+--------+----------+------------------+
| name                   | label  | property | type             |
+------------------------+--------+----------+------------------+
| person_email_unique    | Person | email    | UNIQUENESS       |
| person_name_not_null   | Person | name     | NOT_NULL         |
| person_age_integer     | Person | age      | TYPE_INTEGER     |
| product_sku_warehouse  | Product| sku,warehouse | UNIQUENESS  |
+------------------------+--------+----------+------------------+
```

## MERGE と制約

`MERGE` は UNIQUE 制約のあるプロパティをキーとして使用するパターンで特に有用です。

```cypher
-- email が一意であることが保証されている場合の UPSERT
MERGE (u:User {email: $email})
ON CREATE SET u.created_at = $now, u.name = $name
ON MATCH SET u.last_login = $now
RETURN u
```

## 制約と自動インデックス

UNIQUE 制約を作成すると、対応するプロパティインデックスが自動的に作成されます。明示的にインデックスを作成する必要はありません。

```cypher
CREATE CONSTRAINT ON (n:Person) ASSERT n.email IS UNIQUE
-- 自動的に Person.email のインデックスが作成される

SHOW INDEXES
-- personEmailUnique_index が表示される
```

## 制約違反のエラーハンドリング

Rust クライアントでは制約違反を `Result` で処理できます。

```rust
match client.execute("CREATE (n:Person {email: \"alice@example.com\"})").await {
    Ok(_) => println!("Created successfully"),
    Err(e) if e.to_string().contains("Unique constraint violation") => {
        println!("Email already exists");
    }
    Err(e) => eprintln!("Error: {}", e),
}
```

## ベストプラクティス

- ビジネスキー（メールアドレス、SKU、ユーザーID など）には UNIQUE 制約を設定する
- 必須フィールドには NOT NULL 制約を設定してデータ品質を維持する
- 型制約を使用してデータの型安全性を確保する
- 制約は `MERGE` と組み合わせて使用することで、アトミックな UPSERT 操作を実現できる
