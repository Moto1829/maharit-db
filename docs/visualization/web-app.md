---
title: Web アプリ
parent: 可視化
nav_order: 0
---

# Web アプリ (`maharit-viz`)

`maharit-viz` バイナリは、ブラウザから MaharitDB に対してクエリを実行し、
結果をテーブルおよびグラフで可視化できる HTTP サーバーです。
`maharit-server` (TCP) のフロントエンドとして、別プロセス（または別コンテナ）で動作します。

```
ブラウザ → maharit-viz (HTTP:8080) → maharit-server (TCP:7687)
              ↕
        静的アセット (HTML/JS)
```

## 主な機能

- **クエリエディタ**: textarea + 実行ボタン（`Ctrl/⌘+Enter` で実行）
- **Table タブ**: Tabulator 6.3 によるテーブル表示
  - カラム単位のフィルタ・ソート
  - 25 / 50 / 100 / 250 行ページネーション
  - カラムドラッグによる順序変更
- **Graph タブ**: cytoscape.js 3.30 によるグラフ表示
  - `<prefix>.id` 列を自動検出してノードグループに分類
  - 同一行に複数のノードグループがあればエッジを生成
  - ラベル候補: `.name` / `.title` / `.label`
  - cose レイアウト + ベジェ曲線エッジ
- **Raw JSON タブ**: API レスポンスをそのまま表示

## 起動方法

### ローカル実行

事前に `cargo build --release -p maharit-server -p maharit-viz` でバイナリを生成しておきます。

```bash
# maharit-server を起動 (port 7687)
./target/release/maharit server --data /tmp/maharit.db

# 別ターミナルで maharit-viz を起動 (port 8080)
./target/release/maharit-viz \
  --bind 0.0.0.0:8080 \
  --server 127.0.0.1:7687
```

ブラウザで <http://localhost:8080> を開きます。

### CLI オプション

| オプション | 既定値 | 説明 |
|-----------|--------|-----|
| `--bind`, `-b` | `0.0.0.0:8080` | HTTP リスナーのバインドアドレス |
| `--server`, `-s` | `127.0.0.1:7687` | 接続先 `maharit-server` の TCP アドレス |
| `--assets`, `-a` | クレート同梱 | 静的アセットを配信するディレクトリ |
| `--help`, `-h` | — | ヘルプを表示 |

### Docker 実行

リポジトリの `docker-compose.yml` に `maharit-viz` サービスが定義されています。

```bash
docker compose up -d
```

| サービス | ホストポート | 用途 |
|---------|-------------|------|
| `maharit-server` | `7687` (TCP) / `9090` (HTTP) | データベース本体 / メトリクス |
| `maharit-viz` | `8080` (HTTP) | Web アプリ |

`maharit-viz` は `depends_on.maharit-server.condition: service_healthy`
でサーバーの起動完了を待ってから立ち上がります。

## HTTP API

`maharit-viz` 自身も以下の REST API を提供します（フロントエンドからも利用）。

### `POST /api/query`

クエリを実行して結果を返します。

```http
POST /api/query
Content-Type: application/json

{ "query": "MATCH (n:Person) RETURN n.id, n.name" }
```

成功時のレスポンス:

```json
{
  "columns": ["n.id", "n.name"],
  "rows": [
    { "n.id": "\"alice\"", "n.name": "\"Alice\"" },
    { "n.id": "\"bob\"",   "n.name": "\"Bob\"" }
  ],
  "elapsed_ms": 3
}
```

エラー時のレスポンス（HTTP 400）:

```json
{ "error": "backend error: server error: Parse error: ..." }
```

`columns` は全行のキー集合を昇順ソートした配列で、各行は欠損カラムが `null` で補完されます。

### `GET /api/info`

接続先サーバーと viz のバージョン情報を返します。

```json
{ "server_addr": "127.0.0.1:7687", "version": "0.2.0" }
```

### `GET /api/health`

ヘルスチェック用エンドポイント。`ok` 文字列のみを返します。

### `GET /`（および任意の静的パス）

`--assets` で指定したディレクトリ配下を `tower-http` の `ServeDir` で配信します。
ルートでは `index.html` が返ります。

## グラフ表示のクエリパターン

Graph タブはクエリ結果から `<prefix>.id` 列を検出することでノードを抽出します。
グラフを可視化したいときは、ノードの ID とエッジ両端を返すクエリを書きます。

### 2 ノードのリレーションシップを描画

```cypher
MATCH (a)-[r]->(b)
RETURN a.id, a.name, b.id, b.name
LIMIT 100
```

- `a.id` と `b.id` を検出してノードグループ `a`, `b` を生成
- 同一行で 2 グループが揃うため `a → b` のエッジが張られる
- `.name` 列がノードラベルとして利用される

### ノードのみを表示

```cypher
MATCH (n:Person)
RETURN n.id, n.name
LIMIT 50
```

- ノードグループ `n` のみが検出され、エッジなしのグラフが描画される

### 注意点

- `n.id` 列が存在しない場合は Graph タブで「グラフ表示には `<prefix>.id` を含むクエリが必要です」と案内されます。
- 現在の検出ロジックは簡易ヒューリスティックです。複雑なグラフ可視化が必要な場合は
  [DOT / SVG エクスポート](export.md) と組み合わせて Graphviz / 外部ツールを利用してください。

## アーキテクチャ

- バックエンド: `axum` 0.7（Rust 2024 edition）
- フロントエンド: Vanilla JS + Tabulator + cytoscape.js（いずれも CDN ロード）
- 通信: `maharit-client` クレートを使用して `maharit-server` の TCP プロトコル
  （4 バイト長プレフィックス + JSON）で接続
- 静的アセット: `tower-http::services::ServeDir` で外部ディレクトリを配信

ソースコード:

- `crates/maharit-viz/src/web.rs` — axum サーバーと API ハンドラ
- `crates/maharit-viz/src/bin/viz.rs` — CLI エントリーポイント
- `crates/maharit-viz/assets/index.html` — フロントエンド SPA
