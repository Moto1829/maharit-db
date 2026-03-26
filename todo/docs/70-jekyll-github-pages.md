# Task 70: mdBook を廃止して Jekyll (just-the-docs) に移行

## 概要
mdBook を廃止し、Jekyll + just-the-docs テーマで GitHub Pages に公開する。
既存の docs/src/ コンテンツを docs/ に移動し、フロントマターを追加。

## 実装内容
- [x] docs/_config.yml 作成 (just-the-docs テーマ)
- [x] docs/Gemfile 作成
- [x] docs/src/ の全 .md ファイルに Jekyll フロントマターを追加
- [x] docs/src/ → docs/ にファイルを移動
- [x] 各セクションの親ページ (index.md) を作成
- [x] .github/workflows/docs.yml を Jekyll + actions/deploy-pages 方式に更新
- [x] docs/book.toml と docs/src/SUMMARY.md を削除

## 対象ファイル
- docs/_config.yml (新規)
- docs/Gemfile (新規)
- docs/index.md (introduction.md の内容を移動)
- docs/quickstart.md
- docs/architecture.md
- docs/cypher/ ディレクトリ
- docs/functions/ ディレクトリ
- docs/indexes/ ディレクトリ
- docs/operations/ ディレクトリ
- docs/advanced/ ディレクトリ
- docs/api/ ディレクトリ
- docs/algorithms/ ディレクトリ
- docs/visualization/ ディレクトリ
- .github/workflows/docs.yml (更新)
