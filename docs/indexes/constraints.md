---
title: スキーマ制約
parent: インデックス・制約
nav_order: 3
---

# スキーマ制約

スキーマ制約を定義することで、データの整合性を維持できます。制約に違反する操作はエラーとなります。

## CREATE CONSTRAINT

構文は `CREATE CONSTRAINT <制約名> FOR (<変数>:<ラベル>) REQUIRE <条件>` です（Neo4j 5 系と同じ形式）。制約名は必須で、`DROP CONSTRAINT` で削除するときに使います。

{: .note }
> Neo4j 3.x の旧構文 `CREATE CONSTRAINT ON (n:Label) ASSERT ...` / `DROP CONSTRAINT ON ... ASSERT ...` には対応していません。`FOR ... REQUIRE` 構文に書き換えてください。

### UNIQUE 制約

特定のラベルのノードで、指定プロパティの値が一意であることを保証します。

```cypher
// 単一プロパティの UNIQUE 制約
CREATE CONSTRAINT personEmailUnique FOR (n:Person) REQUIRE n.email IS UNIQUE

// 複合 UNIQUE 制約（複数プロパティの組み合わせが一意）
CREATE CONSTRAINT productSkuWarehouse FOR (n:Product) REQUIRE (n.sku, n.warehouse) IS UNIQUE
```

UNIQUE 制約が存在する場合、重複する値を持つノードを作成しようとするとエラーになります：

```cypher
CREATE (n:Person {email: "alice@example.com"})
// 2 回目の実行はエラー: unique constraint violation: personEmailUnique - property 'email' ...
```

### NOT NULL 制約

特定のプロパティが `null` になれないことを保証します。

```cypher
CREATE CONSTRAINT personNameNotNull FOR (n:Person) REQUIRE n.name IS NOT NULL

CREATE CONSTRAINT productPriceNotNull FOR (n:Product) REQUIRE n.price IS NOT NULL
```

### 型制約

プロパティの値が特定の型であることを保証します。型は `IS :: <型>` で指定します。

```cypher
// 整数型
CREATE CONSTRAINT personAgeInt FOR (n:Person) REQUIRE n.age IS :: INTEGER

// 文字列型
CREATE CONSTRAINT personNameString FOR (n:Person) REQUIRE n.name IS :: STRING

// 真偽値型
CREATE CONSTRAINT settingEnabledBool FOR (n:Setting) REQUIRE n.enabled IS :: BOOLEAN

// 浮動小数点型
CREATE CONSTRAINT productPriceFloat FOR (n:Product) REQUIRE n.price IS :: FLOAT
```

サポートされる型: `INTEGER`（`INT`）、`FLOAT`（`DOUBLE`）、`STRING`、`BOOLEAN`（`BOOL`）

### ラベル制約

あるラベルを持つノードが、別のラベルも必ず持つことを保証します。

```cypher
// Employee ラベルのノードは Person ラベルも持たなければならない
CREATE CONSTRAINT employeeIsPerson FOR (n:Employee) REQUIRE n:Person
```

### リレーションシップ端点制約

指定タイプのリレーションシップが、決まったラベルのノード同士を結ぶことを保証します。

```cypher
// WORKS_AT は Person から Company へのみ張れる
CREATE CONSTRAINT worksAtEndpoints FOR (p:Person)-[r:WORKS_AT]->(c:Company)
```

## DROP CONSTRAINT

制約名を指定して削除します。

```cypher
DROP CONSTRAINT personEmailUnique
```

## SHOW CONSTRAINTS

現在定義されているすべての制約を表示します。

```cypher
SHOW CONSTRAINTS
```

出力例:

```
+---------------------+---------+----------------+--------------+
| name                | label   | properties     | type         |
+---------------------+---------+----------------+--------------+
| personEmailUnique   | Person  | email          | UNIQUE       |
| personNameNotNull   | Person  | name           | NOT NULL     |
| personAgeInt        | Person  | age            | TYPE(INTEGER)|
| productSkuWarehouse | Product | sku, warehouse | UNIQUE       |
+---------------------+---------+----------------+--------------+
```

## MERGE と制約

`MERGE` は UNIQUE 制約のあるプロパティをキーとして使用するパターンで特に有用です。

```cypher
// email が一意であることが保証されている場合の UPSERT
MERGE (u:User {email: $email})
ON CREATE SET u.created_at = $now, u.name = $name
ON MATCH SET u.last_login = $now
RETURN u
```

## 制約とインデックス

UNIQUE 制約を作成しても、プロパティインデックスは自動では作成されません。制約対象のプロパティで検索する場合は、明示的にインデックスを作成してください。

```cypher
CREATE CONSTRAINT personEmailUnique FOR (n:Person) REQUIRE n.email IS UNIQUE

CREATE INDEX ON :Person(email)
```

## 制約違反のエラーハンドリング

Rust クライアントでは制約違反を `Result` で処理できます。

```rust
match client.execute("CREATE (n:Person {email: \"alice@example.com\"})").await {
    Ok(_) => println!("Created successfully"),
    Err(e) if e.to_string().contains("unique constraint violation") => {
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
