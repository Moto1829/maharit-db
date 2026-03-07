# ドキュメント（mdBook）

## 概要
maharit-db の公式ドキュメントを mdBook で作成し、公開できる状態にする。
ユーザー向けのガイド・リファレンス・チュートリアルをカバーする。

## ドキュメント構成

### はじめに
- [x] はじめに（Introduction）: maharit-db の概要・特徴
- [x] クイックスタート: インストール・起動・最初のクエリ
- [x] アーキテクチャ概要: クレート構成・データフロー図

### Cypher クエリ言語リファレンス
- [x] 基本構文: MATCH / CREATE / DELETE / RETURN
- [x] WHERE 句・フィルタリング
- [x] SET / REMOVE / MERGE
- [x] UNWIND / WITH / UNION
- [x] FOREACH / サブクエリ（CALL {}）
- [x] ORDER BY / LIMIT / SKIP
- [x] パラメータ（$param 構文）

### 関数リファレンス
- [x] 文字列関数（toLower, toUpper, trim, split 等）
- [x] 数学関数（abs, ceil, floor, round 等）
- [x] リスト操作（head, tail, range, reduce 等）
- [x] 述語関数（all, any, none, single, exists, isEmpty）
- [x] 集計関数（COUNT, SUM, AVG, MAX, MIN, COLLECT 等）
- [x] 全文検索（CONTAINS, フレーズ検索, ファジー検索）
- [x] 日本語全文検索（形態素解析）

### インデックス・制約
- [x] プロパティインデックスの作成・削除
- [x] 全文検索インデックス（CREATE FULLTEXT INDEX）
- [x] スキーマ制約（UNIQUE, NOT NULL, 型制約, ラベル制約）

### サーバー・運用
- [x] サーバーの設定（TOML 設定ファイル）
- [x] TLS/SSL 設定
- [x] 認証・ロール管理（CREATE USER 等）
- [x] ラベル/プロパティ単位のアクセス制御
- [x] メトリクス・ヘルスチェック（Prometheus / /health）
- [x] OpenTelemetry トレーシング
- [x] バックアップ・リストア
- [x] スケジュールバックアップ
- [x] 増分バックアップ
- [x] ポイントインタイムリカバリ（WAL適用）

### 高度なトピック
- [x] トランザクション（BEGIN / COMMIT / ROLLBACK）
- [x] MVCC（スナップショット分離）
- [x] レプリケーション（リーダー/フォロワー）
- [x] シャーディング（maharit-cluster）
- [x] クエリ最適化（EXPLAIN / PROFILE）

### API リファレンス
- [x] Rust クライアント（maharit-client）の使い方
- [x] TCP プロトコル仕様
- [x] Python クライアント（maharit-python）の使い方
- [x] Python クライアント: 同期 / 非同期 API
- [x] Python クライアント: pandas DataFrame 連携

### グラフアルゴリズム
- [x] 最短経路（shortest_path）
- [x] 全経路探索（all_paths）
- [x] PageRank / 中心性指標
- [x] 連結成分・強連結成分
- [x] ラベル伝播法（コミュニティ検出）

### 可視化
- [x] DOT 形式出力（Graphviz）
- [x] SVG エクスポート（力学モデルレイアウト）
- [x] WebSocket リアルタイム表示

## mdBook セットアップ
- [x] `docs/` ディレクトリに `book.toml` を作成
- [x] `docs/src/` に各章の Markdown を作成
- [x] `docs/src/SUMMARY.md` に目次を定義
- [ ] GitHub Actions で自動ビルド・公開（gh-pages）

## book.toml 例
```toml
[book]
title = "maharit-db ドキュメント"
authors = ["maharit-db contributors"]
language = "ja"
multilingual = false
src = "src"

[output.html]
theme = "defaults"
default-theme = "navy"
git-repository-url = "https://github.com/example/maharit-db"
edit-url-template = "https://github.com/example/maharit-db/edit/main/docs/{path}"
```

## 対象ディレクトリ
`docs/`（リポジトリルート直下に新規作成）

## 依存
- `mdbook` コマンドのインストール（`cargo install mdbook`）
- 全機能タスクが完了していること
