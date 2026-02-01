# MaharitDB

Rustで実装されたグラフデータベース。Cypherライクなクエリ言語をサポートし、高速なグラフ探索とデータ永続化を提供します。

## 特徴

- **Cypherライクなクエリ言語** - `MATCH`, `CREATE`, `DELETE`, `SET` などのクエリをサポート
- **グラフアルゴリズム** - PageRank、最短経路、中心性指標、連結成分など
- **永続化** - WAL（Write-Ahead Logging）による耐障害性
- **TCPサーバー** - ネットワーク経由でのクエリ実行
- **クライアントライブラリ** - 非同期/同期API、コネクションプール対応
- **インポート/エクスポート** - CSV、JSON、GraphML形式をサポート

## インストール

```bash
git clone https://github.com/Moto1829/maharit-db.git
cd maharit-db
cargo build --release
```

## クイックスタート

### REPL

```bash
cargo run
```

```cypher
> CREATE (alice:Person {name: "Alice", age: 30})
> CREATE (bob:Person {name: "Bob", age: 25})
> MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"}) CREATE (a)-[:KNOWS]->(b)
> MATCH (n:Person) RETURN n.name, n.age
```

### TCPサーバー

```bash
cargo run --bin maharit-server
```

### クライアント

```rust
use maharit_client::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Client::connect("localhost:7687").await?;

    client.execute("CREATE (n:Person {name: \"Alice\"})").await?;

    let result = client.query("MATCH (n:Person) RETURN n.name").await?;
    for row in &result.rows {
        println!("{:?}", row);
    }

    Ok(())
}
```

## クレート構成

| クレート | 説明 | 行数 |
|---------|------|-----:|
| `maharit-core` | グラフデータ構造、アルゴリズム | 2,874 |
| `maharit-query` | クエリパーサー、エグゼキュータ | 3,273 |
| `maharit-storage` | 永続化、WAL、トランザクション | 2,127 |
| `maharit-server` | TCPサーバー | 1,293 |
| `maharit-client` | クライアントライブラリ | 1,115 |
| `maharit-io` | インポート/エクスポート | 1,887 |
| `maharit-viz` | グラフ可視化 | 487 |

**合計: 14,467行**

## クエリ言語

### ノード操作

```cypher
-- ノード作成
CREATE (n:Person {name: "Alice", age: 30})

-- ノード検索
MATCH (n:Person) WHERE n.age > 25 RETURN n

-- プロパティ更新
MATCH (n:Person {name: "Alice"}) SET n.age = 31

-- ノード削除
MATCH (n:Person {name: "Alice"}) DELETE n
```

### エッジ操作

```cypher
-- エッジ作成
MATCH (a:Person {name: "Alice"}), (b:Person {name: "Bob"})
CREATE (a)-[:KNOWS {since: 2020}]->(b)

-- 可変長パス
MATCH (a)-[:KNOWS*2..5]->(b) RETURN a, b
```

### 集約関数

```cypher
MATCH (n:Person) RETURN COUNT(n), AVG(n.age), MAX(n.age)
```

## グラフアルゴリズム

```rust
use maharit_core::{Graph, algorithms::*};

let graph = Graph::new();
// ... ノードとエッジを追加 ...

// PageRank
let scores = pagerank(&graph, &PageRankConfig::default());

// 最短経路
let path = dijkstra(&graph, source_id, target_id);

// 中心性
let centrality = betweenness_centrality(&graph);

// 連結成分
let components = connected_components(&graph);
```

## データ形式

### CSV

```rust
use maharit_io::{CsvImporter, CsvExporter};

// インポート
let stats = CsvImporter::import_nodes(&mut graph, nodes_csv)?;
CsvImporter::import_edges(&mut graph, edges_csv, &stats.id_map)?;

// エクスポート
CsvExporter::export_nodes(&graph, &mut output)?;
```

### JSON

```rust
use maharit_io::{JsonImporter, JsonExporter};

// ノード配列 + エッジ配列形式
JsonImporter::import(&mut graph, reader)?;
JsonExporter::export(&graph, writer)?;

// 隣接リスト形式
JsonImporter::import_adjacency(&mut graph, reader)?;
JsonExporter::export_adjacency(&graph, writer)?;
```

### GraphML

```rust
use maharit_io::{GraphMlImporter, GraphMlExporter};

GraphMlImporter::import(&mut graph, reader)?;
GraphMlExporter::export(&graph, writer)?;
```

## Docker

```bash
docker-compose up -d
```

## ライセンス

MIT License
