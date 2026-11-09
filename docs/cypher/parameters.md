---
title: 'パラメータ（$param 構文）'
parent: Cypher クエリ言語
nav_order: 7
---

# パラメータ（$param 構文）

パラメータ化クエリを使用することで、クエリ文字列を動的に構築することなく、値を安全にクエリに渡せます。SQL インジェクションに相当する攻撃の防止にも有効です。

## 基本的な使い方

パラメータは `$` プレフィックスで参照します。

```cypher
// パラメータを使ったノード検索
MATCH (n:Person {name: $name})
RETURN n

// WHERE 句でパラメータを使用
MATCH (n:Person)
WHERE n.age >= $min_age AND n.age <= $max_age
RETURN n.name, n.age
```

パラメータの値は実行時に渡します（JSON 形式）:
```json
{
  "name": "Alice",
  "min_age": 20,
  "max_age": 40
}
```

## 使用できる場所

パラメータはほぼすべての値が使用できる場所で利用可能です。

```cypher
// プロパティの値
MATCH (n:Person {name: $name}) RETURN n

// CREATE 時のプロパティ値
CREATE (n:Person {name: $name, age: $age})

// SET でのプロパティ更新
MATCH (n:Person {name: $name})
SET n.age = $new_age

// LIMIT と SKIP
MATCH (n:Person) RETURN n SKIP $offset LIMIT $count

// リストパラメータ
MATCH (n:Person)
WHERE n.city IN $cities
RETURN n.name
```

リストパラメータの例:
```json
{
  "cities": ["Tokyo", "Osaka", "Kyoto"]
}
```

## マッププロパティの展開

パラメータとしてマップを渡し、プロパティとして展開する構文です。

```cypher
// マップパラメータでノードを作成
CREATE (n:Person $props)
RETURN n
```

パラメータ:
```json
{
  "props": {
    "name": "Alice",
    "age": 30,
    "city": "Tokyo"
  }
}
```

同じ形はリレーションシップ（`-[r:KNOWS $rel]->`）、`MATCH`、`MERGE` でも使えます。`MATCH` / `MERGE` では、マップの全キーが等価条件になります。

```cypher
MATCH (n:Person $props)
RETURN n.name
```

`SET` でマップパラメータを使うと、プロパティをまとめて更新できます。`+=` は既存のプロパティを残し、`=` はマップに無いプロパティを削除します。

```cypher
MATCH (n:Person {name: $name})
SET n += $changes
```

### パラメータのキーを参照する

マップパラメータのキーは `$props.name` または `$props['name']` で参照できます。存在しないキーは `null` になります。

```cypher
CREATE (n:Person {name: $props.name, city: $props.address.city})
```

## UNWIND とパラメータの組み合わせ

リストパラメータを `UNWIND` で展開してバルクインサートができます。

```cypher
UNWIND $persons AS person
CREATE (n:Person {
  name: person.name,
  age: person.age,
  city: person.city
})
```

パラメータ:
```json
{
  "persons": [
    {"name": "Alice", "age": 30, "city": "Tokyo"},
    {"name": "Bob", "age": 25, "city": "Osaka"},
    {"name": "Charlie", "age": 35, "city": "Kyoto"}
  ]
}
```

## Rust クライアントでの使用

```rust
use maharit_client::Client;
use std::collections::HashMap;
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Client::connect("127.0.0.1:7687").await?;

    // パラメータの構築
    let mut params: HashMap<String, Value> = HashMap::new();
    params.insert("name".to_string(), json!("Alice"));
    params.insert("min_age".to_string(), json!(25));

    // パラメータ付きクエリの実行
    let result = client.query_with_params(
        "MATCH (n:Person {name: $name}) WHERE n.age >= $min_age RETURN n.name, n.age",
        params
    ).await?;

    for row in &result.rows {
        println!("{:?}", row);
    }

    Ok(())
}
```

## Python クライアントでの使用

```python
from maharit import Client

with Client.connect("localhost:7687") as client:
    result = client.query(
        "MATCH (n:Person {name: $name}) WHERE n.age >= $min_age RETURN n.name, n.age",
        {"name": "Alice", "min_age": 25},
    )
```

TCP で直接送る場合は、`query` リクエストの `params` フィールドに入れます（[TCP プロトコル仕様](../api/protocol.md)）。

## パラメータの型

パラメータとして渡せる型は次の通りです。

| Rust/JSON の型 | Cypher での型 |
|----------------|--------------|
| `null` | `null` |
| `bool` | Boolean |
| `i64` / `u64` | Integer |
| `f64` | Float |
| `String` | String |
| `Array` | List |
| `Object` | Map |

## 注意事項

- パラメータ名には英数字とアンダースコアを使用してください
- プロパティキー（`n.$key` のような形式）はパラメータ化できません
- ラベル名やリレーションシップ型もパラメータ化できません（クエリの構造はコンパイル時に決定される必要があります）
- クエリが参照しているパラメータを渡さないとエラーになります（`null` にはなりません）
- リストやマップの値は `IN $names` や `UNWIND $rows` などで使えますが、現在はプロパティの値として保存できません
