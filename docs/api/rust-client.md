---
title: Rust クライアント
parent: API リファレンス
nav_order: 1
---

# Rust クライアント

`maharit-client` クレートは MaharitDB サーバーに接続するクライアントです。非同期クライアント（`Client`、tokio）、同期クライアント（`sync::SyncClient`）、コネクションプール（`pool::ConnectionPool`）があります。

## インストール

crates.io には公開していないので、Git リポジトリから依存関係に追加します。

```toml
[dependencies]
maharit-client = { git = "https://github.com/Moto1829/maharit-db" }
tokio = { version = "1", features = ["full"] }
serde_json = "1"
```

## 非同期クライアント（Client）

### 接続

```rust
use maharit_client::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Client::connect("127.0.0.1:7687").await?;
    client.ping().await?;
    println!("Connected to MaharitDB");
    Ok(())
}
```

### クエリの実行

```rust
// 結果を使わないクエリ
client.execute("CREATE (n:Person {name: \"Alice\", age: 30})").await?;

// 結果を返すクエリ
let result = client.query("MATCH (n:Person) RETURN n.name AS name, n.age AS age").await?;
println!("Rows returned: {}", result.row_count());
```

### 結果の処理

`QueryResult::rows` は行の `Vec` で、各行は「列名 → 値」の `HashMap<String, serde_json::Value>` です。

```rust
for row in &result.rows {
    let name = row.get("name").and_then(|v| v.as_str()).unwrap_or_default();
    let age = row.get("age").and_then(|v| v.as_i64()).unwrap_or(0);
    println!("{name}: {age}");
}

// 1 行目の 1 列だけ欲しいとき
let count = client
    .query("MATCH (n:Person) RETURN count(n) AS c")
    .await?
    .get_single("c")
    .and_then(|v| v.as_i64());
```

### パラメータ付きクエリ

値はクエリ文字列に埋め込まず、パラメータ（`Params` = `HashMap<String, serde_json::Value>`）で渡します。

```rust
use maharit_client::Params;
use serde_json::json;

let mut params = Params::new();
params.insert("name".to_string(), json!("Alice"));
params.insert("min_age".to_string(), json!(25));

let result = client
    .query_with_params(
        "MATCH (n:Person {name: $name}) WHERE n.age >= $min_age RETURN n.name AS name",
        params,
    )
    .await?;
```

### 一括投入

```rust
let mut params = Params::new();
params.insert(
    "persons".to_string(),
    json!([
        {"name": "Alice", "age": 30},
        {"name": "Bob", "age": 25},
        {"name": "Charlie", "age": 35}
    ]),
);

client
    .execute_with_params(
        "UNWIND $persons AS person CREATE (n:Person {name: person.name, age: person.age})",
        params,
    )
    .await?;
```

### 結果を分けて受け取る（ストリーミング）

```rust
let mut stream = client.query_stream("MATCH (n:Person) RETURN n.name AS name", 1000).await?;
while let Some(chunk) = stream.next_chunk().await? {
    for row in chunk {
        println!("{:?}", row.get("name"));
    }
}
```

### トランザクション

```rust
let tx = client.begin().await?;
client.query_in_tx("CREATE (:Item {id: 1})", Some(tx)).await?;
client.commit(tx).await?; // 取り消すときは client.rollback(tx)
```

動作の詳細は [トランザクション](../advanced/transactions.md) を参照してください。

## 同期クライアント（SyncClient）

非同期ランタイムを使わないプログラムでは、同期クライアントを使います（内部で tokio ランタイムを持ちます）。

```rust
use maharit_client::sync::SyncClient;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = SyncClient::connect("127.0.0.1:7687")?;

    client.execute("CREATE (n:Person {name: \"Bob\"})")?;

    let result = client.query("MATCH (n:Person) RETURN n.name")?;
    println!("Found {} persons", result.row_count());
    Ok(())
}
```

## コネクションプール

複数のタスクから同時に使うときは、コネクションプールを使います。

```rust
use maharit_client::pool::{ConnectionPool, PoolConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = ConnectionPool::new(
        "127.0.0.1:7687",
        PoolConfig {
            min_connections: 2,
            max_connections: 10,
            ..Default::default()
        },
    )
    .await?;

    // プールから接続を取り出す（ドロップするとプールに戻る）
    let mut conn = pool.get().await?;
    let result = conn.query("MATCH (n:Person) RETURN count(n) AS c").await?;
    println!("{:?}", result.get_single("c"));
    Ok(())
}
```

## TLS 接続

サーバーを `--tls-cert` / `--tls-key` 付きで起動している場合は、`connect_tls` で接続します（[TLS/SSL 設定](../operations/tls.md)）。

```rust
use maharit_client::{Client, TlsClientConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 第 1 引数は証明書を検証するホスト名。with_ca_cert を省略すると公的な CA で検証する
    let tls = TlsClientConfig::new("secure-db.example.com").with_ca_cert("/path/to/ca.crt");
    let mut client = Client::connect_tls("secure-db.example.com:7687", tls).await?;
    client.ping().await?;
    Ok(())
}
```

TLS 接続では `reconnect()` は使えません。切断されたら `connect_tls` で接続し直してください。

## 認証

サーバーを `--require-auth` で起動している場合は、接続後に `login` します。以降のリクエストにはセッショントークンが自動で付きます（[認証・ロール管理](../operations/auth.md)）。

```rust
let mut client = Client::connect("127.0.0.1:7687").await?;
let info = client.login("alice", "password123").await?;
println!("role: {}, expires at: {}", info.role, info.expires_at);
```

## エラーハンドリング

クエリのエラーや認証エラーは `ClientError::Server(メッセージ)` で返ります。接続が切れた場合は `ClientError::ConnectionClosed` / `ClientError::Connection` です。

```rust
use maharit_client::{Client, ClientError};

match client.execute("CREATE (n:Person {email: \"dup@example.com\"})").await {
    Ok(()) => println!("Created"),
    Err(ClientError::Server(msg)) if msg.contains("unique constraint violation") => {
        println!("Email already exists");
    }
    Err(ClientError::ConnectionClosed | ClientError::Connection(_)) => {
        eprintln!("Lost connection to server");
    }
    Err(e) => return Err(e.into()),
}
```
