---
title: アーキテクチャ概要
nav_order: 3
---

# アーキテクチャ概要

MaharitDB は Rust のワークスペース構成を採用し、責任ごとに分離された複数のクレートで構成されています。

## クレート構成

| クレート | 説明 |
|---------|------|
| `maharit-core` | グラフデータ構造、アルゴリズム、全文検索エンジン |
| `maharit-query` | Cypher クエリパーサー、エグゼキュータ、プランナー |
| `maharit-storage` | WAL、永続化、トランザクション、バックアップ |
| `maharit-server` | TCP サーバー、認証、メトリクス、レプリケーション |
| `maharit-client` | 非同期/同期クライアント、コネクションプール |
| `maharit-io` | CSV/JSON/GraphML インポート・エクスポート |
| `maharit-viz` | DOT/SVG 可視化 |

## 各クレートの詳細

### maharit-core

グラフの中心的なデータ構造と操作を提供します。

- **Graph 構造体**: ノード（`Node`）とエッジ（`Edge`）を `HashMap` で管理
- **トラバーサル**: DFS/BFS、最短経路（Dijkstra）、全経路列挙
- **グラフアルゴリズム**: PageRank、中心性指標（媒介・近接）、連結成分
- **全文検索エンジン**: 転置インデックス、BM25 スコアリング、lindera による日本語形態素解析
- **GraphStats**: ノード数・エッジ数・ラベル別統計

```
maharit-core/src/
├── lib.rs          -- 公開 API のエクスポート
├── graph.rs        -- Graph, Node, Edge の定義
├── traversal.rs    -- トラバーサルと経路探索
├── algorithms.rs   -- PageRank, 中心性, 連結成分
└── fulltext.rs     -- 全文検索インデックス（FulltextManager）
```

### maharit-query

Cypher ライクなクエリ言語の処理を担当します。

- **lexer.rs**: 字句解析器（Token, TokenKind）
- **ast.rs**: 抽象構文木の定義（Statement, Expression, Pattern）
- **parser.rs**: 再帰下降パーサー
- **executor.rs**: クエリの実行エンジン（Executor）
- **planner.rs**: クエリプランの生成（QueryPlan, PlanNode）

クエリの処理フロー：

```
入力文字列
  → Lexer（字句解析）
  → Parser（構文解析）
  → AST（抽象構文木）
  → Executor（実行）
    → Graph 操作
    → 結果返却
```

EXPLAIN/PROFILE 実行時はプランナーが介入します：

```
AST → Planner → QueryPlan → Executor（実測値付き）
```

### maharit-storage

データの永続化とトランザクション管理を担います。

- **WAL（Write-Ahead Log）**: 変更操作をディスクに先行書き込み
- **スナップショット**: グラフ全体のシリアライズ/デシリアライズ
- **トランザクション**: MVCC によるスナップショット分離レベル
- **バックアップ**: gzip 圧縮付きフルバックアップ・増分バックアップ
- **PITR**: WAL ログを使ったポイントインタイムリカバリ

### maharit-server

ネットワーク層とサーバー管理を担当します。

- **TCP サーバー**: tokio 非同期 I/O、TLS 対応（rustls）
- **認証・RBAC**: ユーザー管理、ロールベースアクセス制御
- **細粒度 ACL**: ラベル・プロパティ単位のアクセス制御
- **HTTP サーバー**: `/metrics`、`/health`、`/health/live`、`/health/ready`
- **OpenTelemetry**: 分散トレーシング
- **レプリケーション**: WAL ストリーミング、リーダー/フォロワー構成

### maharit-client

サーバーへの接続クライアントを提供します。

- **非同期クライアント**: `Client`（tokio ベース）
- **同期クライアント**: `SyncClient`（非同期ランタイムをラップ）
- **コネクションプール**: 接続の再利用と管理
- **パラメータ化クエリ**: `$param` 形式のパラメータバインディング

### maharit-io

外部フォーマットとのデータ交換を担当します。

- **CSV インポート/エクスポート**: ノード・エッジの一括入出力
- **JSON インポート/エクスポート**: JSON Lines 形式
- **GraphML**: GraphML XML 形式のサポート

### maharit-viz

グラフの可視化機能と Web フロントエンドを提供します。

- **DOT 出力**: Graphviz の DOT 言語形式でエクスポート
- **SVG エクスポート**: 力学モデルレイアウトによる SVG 生成
- **ASCII レンダラー**: ターミナル向けのテキスト可視化
- **Web アプリ** (`maharit-viz` バイナリ): axum ベースの HTTP サーバー
  + Tabulator (テーブル) / cytoscape.js (グラフ) によるブラウザ可視化

## データフロー

```
クライアント（TCP/TLS）
        ↓
    maharit-server
     ├── 認証・ACL チェック
     ├── クエリ受信
     └── maharit-query へ転送
              ↓
         Executor
          ├── maharit-core（Graph 操作）
          └── maharit-storage（WAL 書き込み）
              ↓
         結果を返却
              ↑
    maharit-server
        ↑
クライアント（結果受信）
```

## 依存関係グラフ

```
maharit-server
  ├── maharit-query
  │     └── maharit-core
  ├── maharit-storage
  │     └── maharit-core
  └── maharit-client（テスト用）

maharit-io
  └── maharit-core

maharit-viz
  ├── maharit-core
  └── maharit-client（Web アプリのみ）
```

## メモリモデル

グラフデータはインメモリで管理されます。`Graph` 構造体は以下の `HashMap` を保持します：

- `nodes: HashMap<NodeId, Node>`: ノードの格納
- `edges: HashMap<EdgeId, Edge>`: エッジの格納
- `adjacency: HashMap<NodeId, Vec<EdgeId>>`: 出力エッジのインデックス
- `reverse_adjacency: HashMap<NodeId, Vec<EdgeId>>`: 入力エッジのインデックス
- `label_index: HashMap<String, HashSet<NodeId>>`: ラベルによる逆引きインデックス

永続化はバックグラウンドで WAL とスナップショットによって行われます。

## 永続化モード

MaharitDB は使用する層によって永続化の扱いが異なります。

| 層 | モード | 説明 |
|----|--------|------|
| ライブラリ / API（`maharit-core`） | オンメモリ | `Graph::new()` はファイルを必要とせず、プロセス終了でデータは消える |
| サーバー（`maharit-server`） | 常にファイル永続化 | 起動時に必ずデータファイルを使用する（SQLite と同様） |

### ライブラリとして使う場合（オンメモリ）

`maharit-core` を直接使う場合は `Graph::new()` がインメモリグラフを提供します。永続化は不要な用途（テスト、一時的なグラフ処理、組み込み利用）に適しています。

```rust
use maharit_core::Graph;

let mut graph = Graph::new(); // ファイル不要・オンメモリ
```

### サーバーとして使う場合（常に永続化）

`maharit-server` はファイルパスを指定して起動します。ファイルが存在すればロード、なければ新規作成します。

```bash
# ファイルパスを指定して起動（デフォルト: maharit.db）
maharit server --data /var/lib/maharit/maharit.db

# 環境変数でも指定可能
MAHARIT_DATA=/var/lib/maharit/maharit.db maharit server
```

サーバーは SIGINT / SIGTERM 受信時に自動でデータを保存します。
