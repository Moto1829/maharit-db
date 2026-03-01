# MaharitDB

Rust で実装されたグラフデータベースエンジンです。Cypher ライクなクエリ言語による直感的なグラフ操作と、本番環境向けのサーバー機能を提供します。

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)

## 特徴

- **Cypher ライクなクエリ言語** — `MATCH`、`CREATE`、`MERGE`、`DELETE`、`UNWIND`、`WITH`、`UNION`、`FOREACH`、`CALL {}` サブクエリをサポート
- **BM25 全文検索（日本語対応）** — lindera IPADIC による形態素解析、フレーズ検索・ファジー検索対応
- **グラフアルゴリズム** — PageRank、最短経路（Dijkstra / BFS）、媒介中心性、近接中心性、連結成分
- **WAL による耐障害性** — Write-Ahead Log でクラッシュ後も確実にリカバリ
- **TLS/SSL 対応** — rustls による TLS 1.2/1.3 サポート、証明書ホットリロード
- **認証・RBAC** — ユーザー管理とロールベースアクセス制御（admin / writer / reader）
- **細粒度 ACL** — ラベル・プロパティ単位のアクセス権限制御
- **Prometheus メトリクス** — `/metrics`、`/health` エンドポイントで監視基盤と連携
- **OpenTelemetry トレーシング** — クエリ実行経路の分散トレーシング
- **レプリケーション** — WAL ストリーミングによるリーダー/フォロワー構成
- **バックアップ・PITR** — gzip 圧縮、増分バックアップ、ポイントインタイムリカバリ
- **非同期 Rust クライアント** — tokio ベース、コネクションプール対応

## ドキュメント

完全なドキュメントは [`docs/`](docs/) をご覧ください。
mdBook 形式で提供しており、クエリ言語リファレンス・運用ガイド・API リファレンスを網羅しています。

## インストール

Rust ツールチェーン（1.75 以降）が必要です。

```bash
git clone https://github.com/suzukishimei/maharit-db.git
cd maharit-db
cargo build --release
```

## クイックスタート

### REPL（対話モード）

```bash
cargo run
```

```cypher
> CREATE (alice:Person {name: "Alice", age: 30})
Created 1 node(s).

> CREATE (bob:Person {name: "Bob", age: 25})
Created 1 node(s).

> MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"})
  CREATE (a)-[:KNOWS {since: 2020}]->(b)
Created 1 relationship(s).

> MATCH (n:Person) RETURN n.name, n.age
+--------+-------+
| n.name | n.age |
+--------+-------+
| Alice  | 30    |
| Bob    | 25    |
+--------+-------+
2 rows returned.
```

### TCP サーバー

```bash
# デフォルト設定（127.0.0.1:7687）で起動
cargo run -- server

# すべてのインタフェースにバインド
cargo run -- server --host 0.0.0.0 --port 7687
```

### Rust クライアント

```rust
use maharit_client::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Client::connect("127.0.0.1:7687").await?;

    client.execute("CREATE (n:Person {name: \"Alice\", age: 30})").await?;

    let result = client.query("MATCH (n:Person) RETURN n.name, n.age").await?;
    for row in &result.rows {
        println!("{:?}", row);
    }

    Ok(())
}
```

## クレート構成

| クレート | 説明 |
|---------|------|
| `maharit-core` | グラフデータ構造・アルゴリズム・全文検索エンジン |
| `maharit-query` | Cypher クエリパーサー・エグゼキュータ・プランナー |
| `maharit-storage` | WAL・永続化・トランザクション・バックアップ |
| `maharit-server` | TCP サーバー・認証・メトリクス・レプリケーション |
| `maharit-client` | 非同期/同期クライアント・コネクションプール |
| `maharit-io` | CSV / JSON / GraphML インポート・エクスポート |
| `maharit-viz` | DOT / SVG 可視化・WebSocket リアルタイム表示 |

## グラフアルゴリズム（Rust API）

```rust
use maharit_core::{Graph, algorithms::*};

let graph = Graph::new();
// ... ノードとエッジを追加 ...

let scores     = pagerank(&graph, &PageRankConfig::default());
let path       = dijkstra(&graph, source_id, target_id, "weight");
let centrality = betweenness_centrality(&graph);
let components = connected_components(&graph);
```

## データの入出力

```rust
use maharit_io::{CsvImporter, JsonExporter};

// CSV からインポート
let stats = CsvImporter::import_nodes(&mut graph, nodes_csv)?;
CsvImporter::import_edges(&mut graph, edges_csv, &stats.id_map)?;

// JSON へエクスポート
JsonExporter::export(&graph, writer)?;
```

## Docker

```bash
docker-compose up -d
```

## ライセンス

MIT License — 詳細は [LICENSE](LICENSE) を参照してください。
