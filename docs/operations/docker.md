---
title: Docker での使用
parent: サーバー・運用
nav_order: 8
---

# Docker での使用方法

MaharitDB は Docker を使用して簡単に実行できます。

## クイックスタート

### イメージのビルド

```bash
docker build -t maharit-db .
```

### REPL モードで起動

対話的なREPLモードでMaharitDBを起動します：

```bash
docker run -it maharit-db
```

### サーバーモードで起動

TCPサーバーとして起動し、クライアントからの接続を受け付けます：

```bash
docker run -p 7687:7687 maharit-db server --host 0.0.0.0 --port 7687
```

## docker-compose での使用

### REPL モード

```bash
docker-compose run maharit
```

### サーバーモード

```bash
docker-compose up maharit-server
```

サーバーはポート 7687 でリッスンします。

## 設定オプション

### 環境変数

| 変数名 | 説明 | デフォルト値 |
|--------|------|-------------|
| `MAHARIT_DATA_DIR` | データディレクトリのパス | `/data` |

### サーバーオプション

| オプション | 短縮形 | 説明 | デフォルト値 |
|-----------|--------|------|-------------|
| `--host` | `-h` | バインドするホスト | `127.0.0.1` |
| `--port` | `-p` | リッスンするポート | `7687` |
| `--max-connections` | `-c` | 最大同時接続数 | `100` |

### 使用例

```bash
# カスタムポートで起動
docker run -p 8888:8888 maharit-db server --host 0.0.0.0 --port 8888

# 最大接続数を制限
docker run -p 7687:7687 maharit-db server --host 0.0.0.0 --max-connections 50
```

## データの永続化

ボリュームをマウントしてデータを永続化できます：

```bash
docker run -v maharit-data:/data -p 7687:7687 maharit-db server --host 0.0.0.0
```

docker-compose を使用する場合は、自動的に `maharit-data` ボリュームが作成されます。

## クライアントからの接続

### Rust クライアント

```rust
use maharit_client::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Client::connect("localhost:7687").await?;

    // ノードを作成
    client.execute("CREATE (n:Person {name: \"Alice\"})").await?;

    // クエリを実行
    let result = client.query("MATCH (n:Person) RETURN n.name").await?;
    for row in &result.rows {
        println!("{:?}", row);
    }

    Ok(())
}
```

### 同期クライアント

```rust
use maharit_client::sync::SyncClient;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = SyncClient::connect("localhost:7687")?;

    client.execute("CREATE (n:Person {name: \"Bob\"})")?;

    let result = client.query("MATCH (n:Person) RETURN n.name")?;
    println!("Found {} persons", result.row_count());

    Ok(())
}
```

## トラブルシューティング

### 接続できない場合

1. サーバーが起動しているか確認：
   ```bash
   docker ps
   ```

2. ポートマッピングを確認：
   ```bash
   docker port <container_id>
   ```

3. サーバーログを確認：
   ```bash
   docker logs <container_id>
   ```

### メモリ使用量が多い場合

メモリ制限を設定：
```bash
docker run -m 512m -p 7687:7687 maharit-db server --host 0.0.0.0
```
