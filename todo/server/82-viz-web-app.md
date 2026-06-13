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

### フロントエンド（仮置き / 後続フェーズで本格化）

- [x] `assets/index.html`: 仮置きの単一ページ UI
  - [x] クエリエディタ（textarea + Ctrl/⌘+Enter 実行）
  - [x] 基本テーブル表示
  - [x] エラー表示
  - [ ] Tabulator 統合（カラムフィルタ・ソート・ページネーション）
  - [ ] タブ切り替え（Table / Graph）
  - [ ] cytoscape.js によるネットワークグラフ表示

### インフラ（後続フェーズ）

- [ ] `Dockerfile.viz`: maharit-viz 用 Dockerfile
- [ ] `docker-compose.yml` に `viz` サービスを追記

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
