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

### バックエンド

- [ ] `maharit-viz/Cargo.toml` に axum, tokio, maharit-client を追加
- [ ] `src/server.rs`: Axum HTTP サーバー（静的ファイル配信 + API）
- [ ] `src/api.rs`: `POST /api/query` エンドポイント（maharit-client 経由でクエリ実行、JSON返却）
- [ ] `src/main.rs` または `src/bin/viz.rs`: エントリーポイント

### フロントエンド

- [ ] `src/assets/index.html`: メイン HTML
  - クエリエディタ（textarea + 実行ボタン）
  - タブ切り替え（Table / Graph）
  - Tabulator によるテーブル表示（カラムフィルタ・ソート・ページネーション）
  - cytoscape.js によるネットワークグラフ表示

### インフラ

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
