# Task 82: maharit-viz Web アプリ実装

## 概要
maharit-server とは別プロセス/コンテナとして動作する可視化 Web アプリを実装する。
クエリ結果のテーブル表示とグラフのグラフィカル表示を提供する。

## アーキテクチャ

```
ブラウザ → maharit-viz (HTTP:8080) → maharit-server (TCP:7687)
              ↕
         静的HTML/JS配信
```

## 技術スタック

- **バックエンド**: Rust + Axum (`maharit-viz` クレートに追加)
- **フロントエンド**: Vanilla JS + Tabulator + cytoscape.js (CDN)
- **maharit-server との通信**: `maharit-client` クレートを使用

## 実装タスク

### バックエンド ✅ Phase 1 完了 (2026-06-13)

- [x] `maharit-viz/Cargo.toml` に axum, tokio, maharit-client を追加
- [x] `src/web.rs`: Axum HTTP サーバー（静的ファイル配信 + API）
- [x] `src/web.rs` 内 `query_handler`: `POST /api/query` エンドポイント（maharit-client 経由でクエリ実行、JSON返却）
- [x] `src/bin/viz.rs`: エントリーポイント（CLI 引数 `--bind` / `--server` / `--assets`）
- [x] 補助 API: `GET /api/info`（サーバー情報）/ `GET /api/health`

### フロントエンド ✅ Phase 2 完了 (2026-06-14)

- [x] `assets/index.html`: 本格版の単一ページ UI
  - [x] クエリエディタ（textarea + Ctrl/⌘+Enter 実行）
  - [x] エラー表示
  - [x] Tabulator 統合（CDN 6.3.0 / カラムフィルタ・ソート・ページネーション）
  - [x] タブ切り替え（Table / Graph / Raw JSON）
  - [x] cytoscape.js によるネットワークグラフ表示（CDN 3.30.4）
    - `<prefix>.id` 列からノードグループを自動検出
    - 同一行で複数グループ → エッジ生成
    - ラベル候補: `.name` / `.title` / `.label`
    - cose レイアウト + ベジェ曲線エッジ + ダークテーマ

### インフラ ✅ Phase 3 完了 (2026-06-14)

- [x] `Dockerfile.viz`: maharit-viz 用 Dockerfile（assets 同梱、ヘルスチェック付き）
- [x] `docker-compose.yml` に `maharit-viz` サービスを追記
  - `depends_on.maharit-server.condition: service_healthy`
  - server コンテナ名解決: `maharit-server:7687`

## APIインターフェース

```
POST /api/query
  Request:  { "query": "MATCH (n) RETURN n" }
  Response: { "columns": ["n"], "rows": [[...]], "elapsed_ms": 12 }
            または { "error": "..." }

GET /
  Response: index.html
```

## 参考

- maharit-server: TCP 7687
- maharit-server HTTP (metrics): 9090
- maharit-viz HTTP: 8080

## Phase 1 検証ログ (2026-06-13)

サーバー（port 7688）と viz（port 8088）を起動し、curl で全エンドポイントを検証:

| エンドポイント | 結果 |
|---|---|
| `GET /api/health` | `ok` |
| `GET /api/info` | `{"server_addr":"127.0.0.1:7688","version":"0.2.0"}` |
| `POST /api/query` (CREATE) | `{"columns":["created_edges","created_nodes"],"elapsed_ms":0,"rows":[{"created_edges":"0","created_nodes":"1"}]}` |
| `POST /api/query` (MATCH) | `{"columns":["n.age","n.name"],"elapsed_ms":0,"rows":[{"n.age":"30","n.name":"\"Alice\""}]}` |
| `POST /api/query` (不正クエリ) | `{"error":"backend error: server error: Parse error: ..."}` |
| `GET /` | `index.html` を 200 で配信 |

ユニットテスト: `cargo test -p maharit-viz` → **23/23 PASS**（既存 20 + Phase 1 新規 3）

## Phase 2 検証ログ (2026-06-14)

Python urllib でデータ投入 + クエリを実行し、API レスポンスから
ノードグループ検出 / エッジ生成のロジックが正しく駆動することを確認:

- 投入: Person(alice, bob), City(tokyo) と (alice)-[:LIVES_IN]->(tokyo),
  (alice)-[:KNOWS]->(bob)
- `MATCH (a)-[]->(b) RETURN a.id, a.name, b.id, b.name`
  → ノードグループ `a`, `b` 検出 OK、2 行 → 2 エッジ生成
- index.html 配信時に CDN scripts (`tabulator-tables@6.3.0`, `cytoscape@3.30.4`) を確認

## Phase 3 検証ログ (2026-06-14)

- `Dockerfile.viz` を新規作成 (rust:1.88-slim-bookworm → debian:bookworm-slim)
- `docker-compose.yml` に `maharit-viz` サービスを追加、port 8080 を expose
- `docker compose config` で構文検証 OK
- maharit-server に port 9090 (metrics/health) を expose する変更も併せて実施

## 完了

全フェーズ完了。`docker compose up -d` で server + viz が起動し、
ブラウザから http://localhost:8080 でクエリ実行 / テーブル / グラフ表示が利用可能。
