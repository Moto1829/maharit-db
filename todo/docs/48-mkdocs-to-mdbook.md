# MkDocs → mdBook 移行

## 概要
MkDocs（mkdocs-material）で構成していたドキュメントを mdBook に移行する。

## 実装内容

- [x] `docs/src/` ディレクトリを作成し、全 `.md` ファイルを移動
- [x] `docs/book.toml` を作成（title, description, src, git-repository-url）
- [x] `docs/src/SUMMARY.md` を作成（mkdocs.yml の nav に対応）
- [x] `.github/workflows/docs.yml` を mdBook 用に更新（peaceiris/actions-mdbook 使用）
- [x] `mkdocs.yml` を削除

## 対象ファイル
- `docs/book.toml`（新規作成）
- `docs/src/SUMMARY.md`（新規作成）
- `docs/src/**/*.md`（docs/ から移動）
- `.github/workflows/docs.yml`（更新）
- `mkdocs.yml`（削除）
