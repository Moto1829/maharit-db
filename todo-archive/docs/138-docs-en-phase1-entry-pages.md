# タスク 138: docs 英語翻訳 第 1 弾 — 入口ページ

前提: タスク 136（i18n 基盤・用語集）が完了していること。
進め方: 日本語 `foo.md` から `foo.en.md` を作る。front matter に `source_commit` を入れる。コード・コマンド・出力例は変えない。用語は `docs/_i18n/GLOSSARY.md` に従い、新しい用語が出てきたら用語集に追加する。

最初に読まれるページから訳す。ここまで終われば「英語でインストールして最初のクエリを実行できる」状態になる。

## 対象（約 25KB）
- [x] `index.md`（ホーム）
- [x] `quickstart.md`
- [x] `architecture.md`
- [x] `cypher/index.md`
- [x] `cypher/basics.md`
- [x] 各セクションの `index.md`（functions / indexes / operations / advanced / api / algorithms / visualization）— 第 2・3 弾までの間、英語ナビで各セクションの概要だけは英語で読めるようにする

## 完了条件
- 上記の `.en.md` があり、`mkdocs build --strict` と `docs_cypher_examples` テストが通る
- `/en/` のトップからクイックスタートまでが英語で読める

## 結果（2026-11-07）
- 12 ページの `.en.md` を追加（index / quickstart / architecture / cypher/index / cypher/basics / 各セクションの index 7 枚）
- 訳す前に日本語の正本を実機・コードと照合し、誤りを直した（別コミット `docs: 入口ページの記述を実装に合わせる`）
  - quickstart の REPL / サーバー出力は実際と異なる例だったので実際の出力に置き換え、存在しない `query_with_params` の例を削除
  - サーバーはディスク上の WAL を書かない（停止時保存のみ）ことを index / architecture / README に明記
  - 見つかった不具合: bug/142（DETACH DELETE の件数）、bug/143（TCP パラメータ未対応・Python の query_with_params のインジェクション）、server/144（サーバーの WAL）
- `docs_cypher_examples` の一致検査で `/* */` ブロックコメントも訳せるようにした
- 未対応: `cypher/basics.md` の `{: .note }`（just-the-docs 記法）は MkDocs では admonition にならない。第 2 弾で日英まとめて `!!! note` に直す
