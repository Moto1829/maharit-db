---
title: クイックスタート
nav_order: 2
---

# クイックスタート

このガイドでは MaharitDB をインストールして最初のグラフクエリを実行するまでの手順を説明します。

## 1. インストール

Rust ツールチェーン（1.75 以降）が必要です。まだインストールしていない場合は [rustup.rs](https://rustup.rs) からインストールしてください。

```bash
# リポジトリをクローン
git clone https://github.com/suzukishimei/maharit-db.git
cd maharit-db

# リリースビルド（推奨）
cargo build --release -p maharit-server

# ビルド後のバイナリ場所
ls target/release/maharit
```

## 2. REPL を起動する

REPL（Read-Eval-Print Loop）モードで対話的にクエリを実行できます。

```bash
# デバッグビルドで REPL を起動
cargo run -p maharit-server

# またはリリースバイナリを直接起動
./target/release/maharit
```

起動すると以下のプロンプトが表示されます。

```
MaharitDB v0.1.0
Type your Cypher query and press Enter. Type 'exit' to quit.
>
```

## 3. 最初のクエリ

### ノードを作成する

```cypher
> CREATE (alice:Person {name: "Alice", age: 30})
Created 1 node(s).

> CREATE (bob:Person {name: "Bob", age: 25})
Created 1 node(s).
```

### リレーションシップを作成する

```cypher
> MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"})
  CREATE (a)-[:KNOWS {since: 2020}]->(b)
Created 1 relationship(s).
```

### データを検索する

```cypher
> MATCH (n:Person) RETURN n.name, n.age
+-------+-----+
| n.name | n.age |
+-------+-----+
| Alice  | 30   |
| Bob    | 25   |
+-------+-----+
2 rows returned.
```

### グラフを走査する

```cypher
> MATCH (p:Person)-[:KNOWS]->(friend:Person)
  RETURN p.name AS person, friend.name AS friend
+--------+--------+
| person | friend |
+--------+--------+
| Alice  | Bob    |
+--------+--------+
1 row returned.
```

### プロパティを更新する

```cypher
> MATCH (n:Person {name: "Alice"})
  SET n.email = "alice@example.com"
Updated 1 node(s).
```

### ノードを削除する

```cypher
-- リレーションシップを含めて削除
> MATCH (n:Person {name: "Bob"})
  DETACH DELETE n
Deleted 1 node(s), 1 relationship(s).
```

## 4. サーバーモードで起動する

TCP サーバーとして起動し、クライアントからネットワーク越しに接続できます。

```bash
# デフォルト設定で起動（127.0.0.1:7687）
cargo run -p maharit-server -- server

# すべてのインタフェースにバインド
cargo run -p maharit-server -- server --host 0.0.0.0 --port 7687

# 最大接続数を指定
cargo run -p maharit-server -- server --host 0.0.0.0 --port 7687 --max-connections 200
```

サーバーが起動すると以下のログが表示されます。

```
{"level":"INFO","message":"MaharitDB server starting","host":"0.0.0.0","port":7687}
{"level":"INFO","message":"Server listening","addr":"0.0.0.0:7687"}
```

## 5. クライアントから接続する

### Rust クライアント

`Cargo.toml` に依存関係を追加します（`/path/to/maharit-db` はクローンしたリポジトリのパスに置き換えてください）。

```toml
[dependencies]
maharit-client = { path = "/path/to/maharit-db/crates/maharit-client" }
tokio = { version = "1", features = ["full"] }
```

基本的な使い方：

```rust
use maharit_client::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // サーバーに接続
    let mut client = Client::connect("127.0.0.1:7687").await?;

    // ノードを作成
    client.execute("CREATE (n:Person {name: \"Charlie\", age: 35})").await?;

    // クエリを実行して結果を受け取る
    let result = client.query("MATCH (n:Person) RETURN n.name, n.age").await?;
    for row in &result.rows {
        println!("{:?}", row);
    }

    Ok(())
}
```

### パラメータ化クエリ

SQL インジェクション対策として、値を直接クエリに埋め込む代わりにパラメータを使用してください。

```rust
use std::collections::HashMap;
use serde_json::json;

let mut params = HashMap::new();
params.insert("name".to_string(), json!("Alice"));
params.insert("age".to_string(), json!(30));

let result = client.query_with_params(
    "MATCH (n:Person {name: $name}) WHERE n.age >= $age RETURN n",
    params
).await?;
```

## 6. Docker で起動する

Docker を使用する場合は、別途 Docker のインストールが必要です。

```bash
# イメージをビルド
docker build -t maharit-db .

# REPL モードで起動
docker run -it maharit-db

# サーバーモードで起動（ポートを公開）
docker run -p 7687:7687 maharit-db server --host 0.0.0.0 --port 7687

# データを永続化してサーバーを起動
docker run -v maharit-data:/data -p 7687:7687 maharit-db server --host 0.0.0.0
```

## 次のステップ

- [アーキテクチャ概要](./architecture.md) でクレート構成を理解する
- [Cypher 基本構文](./cypher/basics.md) でクエリ言語を学ぶ
- [関数リファレンス](./functions/string.md) で利用可能な関数を確認する
- [サーバー設定](./operations/server-config.md) で本番環境向けの設定を行う
