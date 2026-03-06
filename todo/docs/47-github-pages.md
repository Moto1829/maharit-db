# GitHub Pages でドキュメントを公開

**Status**: Completed

## 概要
mdBook で作成したドキュメントを GitHub Pages で自動ビルド・公開する。
`main` ブランチへの push 時に自動でデプロイされる仕組みを構築する。

## 実装内容

### GitHub Actions ワークフロー
- [ ] `.github/workflows/docs.yml` を作成
- [ ] `docs/` 配下の変更時のみビルドをトリガー
- [ ] `peaceiris/actions-mdbook` で mdbook をインストール・ビルド
- [ ] `peaceiris/actions-gh-pages` で `gh-pages` ブランチにデプロイ

### GitHub リポジトリ設定
- [ ] リポジトリの Settings > Pages > Source を `gh-pages` ブランチに設定
- [ ] 公開 URL の確認（`https://<user>.github.io/maharit-db/`）

### ワークフロー設定例
```yaml
# .github/workflows/docs.yml
name: Deploy mdBook to GitHub Pages

on:
  push:
    branches: [main]
    paths:
      - 'docs/**'
      - '.github/workflows/docs.yml'
  workflow_dispatch:  # 手動実行も可能

permissions:
  contents: write

jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install mdBook
        uses: peaceiris/actions-mdbook@v2
        with:
          mdbook-version: latest

      - name: Build mdBook
        run: mdbook build docs/

      - name: Deploy to GitHub Pages
        uses: peaceiris/actions-gh-pages@v4
        with:
          github_token: ${{ secrets.GITHUB_TOKEN }}
          publish_dir: ./docs/book
          publish_branch: gh-pages
```

### オプション対応
- [ ] カスタムドメインの設定（`docs/CNAME` ファイル）
- [ ] `book.toml` に `git-repository-url` を実際のリポジトリ URL で設定

## 依存
- `46-mdbook-documentation.md` の mdBook セットアップ（`book.toml` と `docs/src/` の作成）が完了していること

## 対象ファイル
- `.github/workflows/docs.yml`（新規作成）
- `docs/book.toml`（`git-repository-url` を更新）
