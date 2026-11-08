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
git clone https://github.com/Moto1829/maharit-db.git
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

起動すると以下のメッセージが表示され、`maharit>` プロンプトでクエリを入力できます。`:help` でコマンド一覧、`:quit` で終了します。

```text
MaharitDB - Graph Database v0.2.0
Type :help for available commands

maharit>
```

## 3. 最初のクエリ

書き込みクエリは、作成・更新・削除した件数を表で返します。

### ノードを作成する

```text
maharit> CREATE (alice:Person {name: "Alice", age: 30})
created_nodes | created_edges
--------------------------------
1 | 0

1 row(s) returned
maharit> CREATE (bob:Person {name: "Bob", age: 25})
created_nodes | created_edges
--------------------------------
1 | 0

1 row(s) returned
```

### リレーションシップを作成する

```text
maharit> MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) CREATE (a)-[:KNOWS {since: 2020}]->(b)
created_nodes | created_edges
--------------------------------
0 | 1

1 row(s) returned
```

### データを検索する

```text
maharit> MATCH (n:Person) RETURN n.name, n.age ORDER BY n.name
n.name | n.age
-----------------
"Alice" | 30
"Bob" | 25

2 row(s) returned
```

### グラフを走査する

```text
maharit> MATCH (p:Person)-[:KNOWS]->(friend:Person) RETURN p.name AS person, friend.name AS friend
person | friend
------------------
"Alice" | "Bob"

1 row(s) returned
```

### プロパティを更新する

```text
maharit> MATCH (n:Person {name: "Alice"}) SET n.email = "alice@example.com"
properties_set
-----------------
1

1 row(s) returned
```

### ノードを削除する

`DETACH DELETE` はノードと、そのノードにつながるリレーションシップをまとめて削除します。

```text
maharit> MATCH (n:Person {name: "Bob"}) DETACH DELETE n
deleted_nodes | deleted_edges
--------------------------------
1 | 0

1 row(s) returned
```

!!! note
    現在の実装では、`DETACH DELETE` で一緒に消えたリレーションシップは `deleted_edges` に数えられません（リレーションシップ自体は削除されています）。

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

デフォルト設定で起動すると、以下のようなログが表示されます（実際には JSON 形式の構造化ログも同時に出力されます）。認証は既定で無効です。本番環境での設定は [認証](./operations/auth.md) を参照してください。

```text
Starting with empty database (will save to maharit.db)
HTTP monitoring server listening on 127.0.0.1:9090
Server listening on 127.0.0.1:7687
WARN: maharit-server authentication is DISABLED. All requests are accepted without sessionToken.
```

`Ctrl+C`（SIGINT）または SIGTERM で停止すると、データをファイルに保存してから終了します。

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

### パラメータ付きクエリ

利用者の入力などの値は、クエリ文字列に埋め込まずにパラメータ（`$name`）で渡してください。値はクエリとは別に送られ、Cypher として解釈されないので、インジェクションを防げます。

```rust
use maharit_client::Params;
use serde_json::json;

let params = Params::from([
    ("name".to_string(), json!("Alice")),
    ("min_age".to_string(), json!(25)),
]);
let result = client
    .query_with_params(
        "MATCH (n:Person {name: $name}) WHERE n.age >= $min_age RETURN n.name, n.age",
        params,
    )
    .await?;
```

詳細は [パラメータ（$param 構文）](./cypher/parameters.md) を参照してください。

## 6. Web UI (`maharit-viz`) で可視化する

`maharit-viz` は `maharit-server` のフロントエンドとして動作する
HTTP サーバーで、ブラウザからクエリ実行 / テーブル表示 / グラフ表示を行えます。

```bash
# サーバーを起動 (port 7687)
./target/release/maharit server --data /tmp/maharit.db

# 別ターミナルで maharit-viz を起動 (port 8080)
cargo run --release -p maharit-viz -- \
  --bind 127.0.0.1:8080 \
  --server 127.0.0.1:7687
```

ブラウザで <http://localhost:8080> を開きます。

- **Table** タブ: Tabulator によるフィルタ・ソート・ページネーション
- **Graph** タブ: cytoscape.js によるネットワークグラフ表示
  （クエリで `<prefix>.id` 列を返すと自動で可視化）
- **Raw JSON** タブ: API レスポンスを生 JSON で確認

詳細は [Web アプリのドキュメント](./visualization/web-app.md) を参照してください。

## 7. Docker で起動する

Docker を使用する場合は、別途 Docker のインストールが必要です。

```bash
# イメージをビルド
docker build -t maharit-db .

# REPL モードで起動
docker run -it maharit-db

# サーバーモードで起動（ポートを公開）
docker run -p 7687:7687 maharit-db server --host 0.0.0.0 --port 7687

# データを永続化してサーバーを起動
docker run -v maharit-data:/data -p 7687:7687 maharit-db server --host 0.0.0.0 --data /data/maharit.db
```

### docker-compose で server + Web UI を同時起動

リポジトリの `docker-compose.yml` には `maharit-server` (port 7687) と
`maharit-viz` (port 8080) のサービスがあらかじめ定義されています。

```bash
docker compose up -d

# ブラウザで http://localhost:8080 にアクセス
```

## 次のステップ

- [アーキテクチャ概要](./architecture.md) でクレート構成を理解する
- [Cypher 基本構文](./cypher/basics.md) でクエリ言語を学ぶ
- [関数リファレンス](./functions/string.md) で利用可能な関数を確認する
- [サーバー設定](./operations/server-config.md) で本番環境向けの設定を行う
- [Web アプリ](./visualization/web-app.md) でブラウザからの可視化方法を確認する
