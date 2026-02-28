# ドキュメント（mdBook）

## 概要
maharit-db の公式ドキュメントを mdBook で作成し、公開できる状態にする。
ユーザー向けのガイド・リファレンス・チュートリアルをカバーする。

## ドキュメント構成

### はじめに
- [ ] はじめに（Introduction）: maharit-db の概要・特徴
- [ ] クイックスタート: インストール・起動・最初のクエリ
- [ ] アーキテクチャ概要: クレート構成・データフロー図

### Cypher クエリ言語リファレンス
- [ ] 基本構文: MATCH / CREATE / DELETE / RETURN
- [ ] WHERE 句・フィルタリング
- [ ] SET / REMOVE / MERGE
- [ ] UNWIND / WITH / UNION
- [ ] FOREACH / サブクエリ（CALL {}）
- [ ] ORDER BY / LIMIT / SKIP
- [ ] パラメータ（$param 構文）

### 関数リファレンス
- [ ] 文字列関数（toLower, toUpper, trim, split 等）
- [ ] 数学関数（abs, ceil, floor, round 等）
- [ ] リスト操作（head, tail, range, reduce 等）
- [ ] 述語関数（all, any, none, single, exists, isEmpty）
- [ ] 集計関数（COUNT, SUM, AVG, MAX, MIN, COLLECT 等）
- [ ] 全文検索（CONTAINS, フレーズ検索, ファジー検索）
- [ ] 日本語全文検索（形態素解析）

### インデックス・制約
- [ ] プロパティインデックスの作成・削除
- [ ] 全文検索インデックス（CREATE FULLTEXT INDEX）
- [ ] スキーマ制約（UNIQUE, NOT NULL, 型制約, ラベル制約）

### サーバー・運用
- [ ] サーバーの設定（TOML 設定ファイル）
- [ ] TLS/SSL 設定
- [ ] 認証・ロール管理（CREATE USER 等）
- [ ] ラベル/プロパティ単位のアクセス制御
- [ ] メトリクス・ヘルスチェック（Prometheus / /health）
- [ ] OpenTelemetry トレーシング
- [ ] バックアップ・リストア
- [ ] スケジュールバックアップ
- [ ] 増分バックアップ
- [ ] ポイントインタイムリカバリ（WAL適用）

### 高度なトピック
- [ ] トランザクション（BEGIN / COMMIT / ROLLBACK）
- [ ] MVCC（スナップショット分離）
- [ ] レプリケーション（リーダー/フォロワー）
- [ ] シャーディング（maharit-cluster）
- [ ] クエリ最適化（EXPLAIN / PROFILE）

### API リファレンス
- [ ] Rust クライアント（maharit-client）の使い方
- [ ] TCP プロトコル仕様
- [ ] Python クライアント（maharit-python）の使い方
- [ ] Python クライアント: 同期 / 非同期 API
- [ ] Python クライアント: pandas DataFrame 連携

### グラフアルゴリズム
- [ ] 最短経路（shortest_path）
- [ ] 全経路探索（all_paths）
- [ ] PageRank / 中心性指標
- [ ] 連結成分・強連結成分
- [ ] ラベル伝播法（コミュニティ検出）

### 可視化
- [ ] DOT 形式出力（Graphviz）
- [ ] SVG エクスポート（力学モデルレイアウト）
- [ ] WebSocket リアルタイム表示

## mdBook セットアップ
- [ ] `docs/` ディレクトリに `book.toml` を作成
- [ ] `docs/src/` に各章の Markdown を作成
- [ ] `docs/src/SUMMARY.md` に目次を定義
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
