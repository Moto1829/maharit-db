# GitHub Pages でドキュメントを公開

## 概要
mdBook で作成したドキュメントを GitHub Pages で自動ビルド・公開する。
`main` ブランチへの push 時に自動でデプロイされる仕組みを構築する。

## 実装内容

### GitHub Actions ワークフロー
- [x] `.github/workflows/docs.yml` を作成
- [x] `docs/` 配下の変更時のみビルドをトリガー
- [x] `peaceiris/actions-mdbook` で mdbook をインストール・ビルド
- [x] ~~`peaceiris/actions-gh-pages` で `gh-pages` ブランチにデプロイ~~ → 方針変更（下記参照）

### 方針変更: `actions/deploy-pages` 方式に移行
現状、GitHub Pages が main ブランチの `docs/` を Jekyll で直接処理してしまい、
mdBook ソース内の Cypher クエリ（`{{name: ...}}`）が Liquid 構文エラーになる。

**採用方針**: `actions/upload-pages-artifact` + `actions/deploy-pages`（GitHub 推奨の現行方式）
- gh-pages ブランチ不要
- mdBook ビルド結果のみをデプロイ


    contents: read
    pages: write
    id-token: write

  jobs:
    deploy:
      environment:
        name: github-pages
        url: ${{ steps.deployment.outputs.page_url }}
      steps:
        - uses: actions/checkout@v4
        - uses: peaceiris/actions-mdbook@v2
          with:
            mdbook-version: latest
        - run: mdbook build docs/
        - uses: actions/configure-pages@v5
        - uses: actions/upload-pages-artifact@v3
          with:
            path: ./docs/book
        - id: deployment
          uses: actions/deploy-pages@v4
  ```

### GitHub リポジトリ設定
- [x] Settings > Pages > Source を **「GitHub Actions」** に変更（「Deploy from a branch」から変更）
- [x] 公開 URL の確認（`https://moto1829.github.io/maharit-db/`）

### オプション対応
- [x] `book.toml` に `git-repository-url` を実際のリポジトリ URL で設定

## 依存
- `46-mdbook-documentation.md` の mdBook セットアップ（`book.toml` と `docs/src/` の作成）が完了していること

## 対象ファイル
- `.github/workflows/docs.yml`（新規作成）
- `docs/book.toml`（`git-repository-url` を更新）
