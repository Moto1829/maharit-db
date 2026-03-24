---
title: Rust クライアント
parent: API リファレンス
nav_order: 1
---

# Rust クライアント

`maharit-client` クレートは MaharitDB サーバーへの接続クライアントを提供します。非同期クライアント（`Client`）と同期クライアント（`SyncClient`）の両方をサポートします。

## インストール

`Cargo.toml` に依存関係を追加します：

```toml
[dependencies]
maharit-client = { path = "/path/to/maharit-db/maharit-client" }
tokio = { version = "1", features = ["full"] }
serde_json = "1"
```

## 非同期クライアント（Client）

### 接続

```rust
use maharit_client::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 基本的な接続
    let mut client = Client::connect("127.0.0.1:7687").await?;

    println!("Connected to MaharitDB");
    Ok(())
}
```

### クエリの実行

```rust
// 書き込み操作（結果を返さない）
client.execute("CREATE (n:Person {name: \"Alice\", age: 30})").await?;

// 読み取りクエリ（結果を返す）
let result = client.query("MATCH (n:Person) RETURN n.name, n.age").await?;

println!("Rows returned: {}", result.rows.len());
for row in &result.rows {
    println!("{:?}", row);
}
```

### パラメータ付きクエリ

```rust
use std::collections::HashMap;
use serde_json::{json, Value};

let mut params: HashMap<String, Value> = HashMap::new();
params.insert("name".to_string(), json!("Alice"));
params.insert("min_age".to_string(), json!(25));

let result = client.query_with_params(
    "MATCH (n:Person {name: $name}) WHERE n.age >= $min_age RETURN n",
    params
).await?;
```

### 結果の処理

```rust
let result = client.query(
    "MATCH (n:Person) RETURN n.name AS name, n.age AS age"
).await?;

for row in &result.rows {
    // 列名でアクセス
    if let Some(name) = row.get("name") {
        println!("Name: {}", name);
    }
    if let Some(age) = row.get("age") {
        println!("Age: {}", age);
    }
}
```

### バルクオペレーション

```rust
let persons = vec![
    json!({"name": "Alice", "age": 30}),
    json!({"name": "Bob", "age": 25}),
    json!({"name": "Charlie", "age": 35}),
];

let mut params = HashMap::new();
params.insert("persons".to_string(), json!(persons));

client.execute_with_params(
    "UNWIND $persons AS person CREATE (n:Person {name: person.name, age: person.age})",
    params
).await?;
```

## 同期クライアント（SyncClient）

非同期ランタイムが使えない環境では同期クライアントを使用します。

```rust
use maharit_client::sync::SyncClient;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = SyncClient::connect("127.0.0.1:7687")?;

    client.execute("CREATE (n:Person {name: \"Bob\"})")?;

    let result = client.query("MATCH (n:Person) RETURN n.name")?;
    println!("Found {} persons", result.row_count());

    for row in &result.rows {
        println!("{:?}", row);
    }

    Ok(())
}
```

## コネクションプール

高スループットが必要な場合はコネクションプールを使用します。

```rust
use maharit_client::pool::Pool;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = Pool::builder()
        .max_size(10)
        .min_idle(2)
        .connect_timeout(std::time::Duration::from_secs(5))
        .build("127.0.0.1:7687")
        .await?;

    // プールからコネクションを取得
    let mut conn = pool.get().await?;
    let result = conn.query("MATCH (n:Person) RETURN count(n)").await?;

    // conn がドロップされると自動的にプールに返却される

    Ok(())
}
```

## TLS 接続

```rust
use maharit_client::ClientBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = ClientBuilder::new("secure-db.example.com:7687")
        .with_tls(true)
        .with_ca_cert("/path/to/ca.crt")  // 自己署名証明書の場合
        .build()
        .await?;

    Ok(())
}
```

## 認証

```rust
let client = ClientBuilder::new("localhost:7687")
    .with_auth("alice", "password123")
    .build()
    .await?;
```

## エラーハンドリング

```rust
use maharit_client::{Client, ClientError};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Client::connect("127.0.0.1:7687").await?;

    match client.execute("CREATE (n:Person {email: \"dup@example.com\"})").await {
        Ok(_) => println!("Created"),
        Err(ClientError::QueryError(msg)) if msg.contains("Unique constraint") => {
            println!("Email already exists");
        }
        Err(ClientError::ConnectionError(_)) => {
            eprintln!("Lost connection to server");
        }
        Err(e) => return Err(e.into()),
    }

    Ok(())
}
```

## 型変換ヘルパー

```rust
// QueryRow から Rust の型に変換
let result = client.query("MATCH (n:Person) RETURN n.name, n.age").await?;

for row in &result.rows {
    let name: String = row.get_string("name").unwrap_or_default();
    let age: i64 = row.get_integer("age").unwrap_or(0);
    println!("{}: {}", name, age);
}
```
