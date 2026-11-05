# タスク 138: docs 英語翻訳 第 1 弾 — 入口ページ

前提: タスク 136（i18n 基盤・用語集）が完了していること。
進め方: 日本語 `foo.md` から `foo.en.md` を作る。front matter に `source_commit` を入れる。コード・コマンド・出力例は変えない。用語は `docs/_i18n/GLOSSARY.md` に従い、新しい用語が出てきたら用語集に追加する。

最初に読まれるページから訳す。ここまで終われば「英語でインストールして最初のクエリを実行できる」状態になる。

## 対象（約 25KB）
- [ ] `index.md`（ホーム）
- [ ] `quickstart.md`
- [ ] `architecture.md`
- [ ] `cypher/index.md`
- [ ] `cypher/basics.md`
- [ ] 各セクションの `index.md`（functions / indexes / operations / advanced / api / algorithms / visualization）— 第 2・3 弾までの間、英語ナビで各セクションの概要だけは英語で読めるようにする

## 完了条件
- 上記の `.en.md` があり、`mkdocs build --strict` と `docs_cypher_examples` テストが通る
- `/en/` のトップからクイックスタートまでが英語で読める
