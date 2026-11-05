# タスク 136: docs / README の英語版提供 — 方針と基盤整備

状態: 完了 (2026-11-06)

## 背景

docs（MkDocs Material、57 ページ・約 217KB）と README.md は日本語のみ。
国外の利用者に向けて英語版も提供したい。一方、開発は日本語で進んでいるため
**日本語を正本（source of truth）とし、英語は翻訳として追従させる**。
英語版を作ったあとに日本語だけが更新され、英語版が古いまま放置されるのを防ぐ仕組みも同時に用意する。

このタスクは全体方針の決定と基盤整備。翻訳作業そのものは 137〜140、鮮度チェックは 141。

## 全体方針

| 項目 | 方針 | 理由 |
|---|---|---|
| 正本 | 日本語 | 開発・レビューが日本語。英語は日本語から翻訳する |
| 仕組み | [mkdocs-static-i18n](https://github.com/ultrabug/mkdocs-static-i18n) の **suffix 構成**（`basics.md` = 日本語、`basics.en.md` = 英語） | 既存ファイルを移動しないので、日本語ページの URL、`docs_cypher_known_failures.txt` のキー、外部からのリンクが変わらない |
| URL | 日本語は現状どおり `/`、英語は `/en/` | 既存 URL を壊さない。英語をルートにしたくなったら後から `default` を入れ替えれば済む |
| 未翻訳ページ | `fallback_to_default: true` で日本語版を表示 | 段階的に翻訳を進められる。英語ナビゲーションが途中で切れない |
| README | `README.md` を英語にし、日本語は `README.ja.md` に移す。両方の先頭に相互リンク | GitHub は `README.md` だけをトップに表示する。国外の利用者が最初に見るのはここ（2026-11-05 ユーザー決定: トップは英語） |
| 用語 | 用語集 `docs/_i18n/GLOSSARY.md` を作り、訳語を固定する | ページごとに訳語が揺れるのを防ぐ（ノード=node、リレーションシップ=relationship、エッジ=edge の使い分けなど） |
| 翻訳手段 | Claude で下訳 → 用語集と照合 → コード例・コマンド・出力は一切変えない | 量が多い（約 217KB）。コード部分は翻訳対象外 |
| 鮮度管理 | 英語ファイルの front matter に、翻訳元の日本語ファイルのコミットを記録（`source_commit: <sha>`）。CI で日本語側に新しい変更があれば警告（タスク 141） | 「英語版が古い」ことを機械的に検出できる |
| 対象外（今回） | CHANGELOG・リリースノート（コミットメッセージが日本語）、コードコメント、CLI のヘルプ（既に英語）、`clients/python` の README・Sphinx（既に英語） | 別途判断する |

## 作業内容

### 1. MkDocs に i18n を導入
- [x] `mkdocs.yml` に `plugins:` を追加（**`search` を明示しないと検索が消える**ので必ず併記）
  ```yaml
  plugins:
    - search:
        lang: [ja, en]
    - i18n:
        docs_structure: suffix
        fallback_to_default: true
        reconfigure_material: true      # 言語切替メニューと theme.language を自動設定
        reconfigure_search: true
        languages:
          - locale: ja
            default: true
            name: 日本語
            build: true
          - locale: en
            name: English
            build: true
            site_name: MaharitDB Documentation
            site_description: High-performance graph database with Cypher queries
            nav_translations:
              ホーム: Home
              クイックスタート: Quick Start
              # … nav の全見出し（基本構文: Basic Syntax など）
  ```
- [x] `theme.language: ja` は plugin 側で切り替わるので削除または plugin 任せにする
- [x] `exclude_docs: _i18n/` を追加（用語集をサイトに出さない）
- [x] `.github/workflows/docs.yml` の `pip install mkdocs-material` に `mkdocs-static-i18n` を追加し、両方バージョンを固定する（`requirements-docs.txt` を作る）
- [x] `mkdocs build --strict` が通ることを確認する。現状 nav にないページ（`advanced/parallelization.md`、`algorithms/cycle-detection.md`、`visualization/ascii.md`、`visualization/web-app.md`）は警告になるので、nav に追加するか `not_in_nav` に入れる
- [x] `overrides/main.html`（gtag）が英語ページにも効くことを確認する

### 2. docs の Cypher パース検査を英語ファイルに対応させる
`crates/maharit-query/tests/docs_cypher_examples.rs` は `docs/` 以下の `.md` をすべて走査するので、`*.en.md` も対象になる。
- [x] known-failures のキーを作るときに `.en.md` を `.md` に正規化し、同じ例を二重に登録しなくて済むようにする
- [x] 英語ファイルの ```cypher ブロックが、対応する日本語ファイルと**同一**であることを検査するテストを追加する（翻訳時にクエリを書き換えてしまう事故を防ぐ。コメント行 `//` は翻訳してよいので比較対象から除く）

### 3. 用語集とスタイルガイド
- [x] `docs/_i18n/GLOSSARY.md` を作成し、訳語を表にする（少なくとも次を決める）
  - グラフ用語: ノード / リレーションシップ / エッジ / ラベル / プロパティ / パス / 隣接
  - クエリ: 句 / 節 / 述語 / 集計 / バインディング / パラメータ / サブクエリ
  - 運用: リーダー / フォロワー / フェイルオーバー / シャード / WAL / グループコミット / 永続化
  - 製品名・固有名は訳さない（MaharitDB、Cypher、BM25、IPADIC、Prometheus など）
- [x] 同じファイルに英語のスタイルを書く: 米国英語、見出しは Sentence case、コード・コマンド・出力例・エラーメッセージは原文のまま、admonition の種類（`!!! note` など）は変えない
- [x] `source_commit` front matter の書式を決める
  ```markdown
  ---
  source_commit: 6da4c93d   # 翻訳元の日本語ファイル（同名の .md）を最後に反映したコミット
  ---
  ```

### 4. CLAUDE.md に運用ルールを追記
- [x] 「docs の英語版」節を追加する
  - 日本語を更新したら、同名の `.en.md` も同じコミットで更新し `source_commit` を上げる。その場で訳せない場合は、`.en.md` の冒頭に「翻訳が古い」旨の admonition を付ける
  - 新しいページを足すときは日本語版だけでもよい（fallback で日本語が表示される）が、`nav_translations` は必ず追加する

## 実施結果 (2026-11-06)

- `mkdocs.yml`: `plugins`（search ja/en + i18n suffix、`fallback_to_default`）、`nav_translations` 49 項目、`exclude_docs: _i18n/`。
  `theme.language` は削除（プラグインが言語ごとに設定）
- nav に無かった 4 ページ（parallelization / cycle-detection / web-app / ascii）を nav に追加。
  `algorithms/index.md` の拡張子なし相対リンク（`./shortest-path` 等）を `.md` リンクに修正
- `requirements-docs.txt`（mkdocs 1.6.1 / mkdocs-material 9.8.0 / mkdocs-static-i18n 1.3.1）、`.gitignore` に `.venv-docs/`・`/site/`
- `docs.yml`: requirements から固定版をインストール、`--strict` ビルド、PR でもビルドのみ実行（デプロイは main / 手動のみ。`pages`・`id-token` 権限は deploy ジョブに限定）
- `docs_cypher_examples.rs`: `.en.md` の known-failure キーを日本語ファイル名に正規化。
  新テスト `translated_cypher_blocks_match_source`（`//` コメントを除いて日本語版と一致、原文が無い翻訳はエラー）と
  `strip_line_comment_keeps_strings`。`_` で始まるディレクトリは走査しない
- `docs/_i18n/GLOSSARY.md`: 進め方・スタイル・訳さないもの・用語表（グラフ / クエリ / ストレージ・運用）
- CLAUDE.md に「docs の英語版（i18n）」節

### 検証
- `mkdocs build --strict` 成功。`/en/` 配下に全ページ生成（未翻訳は日本語にフォールバック）、`<html lang="en">`、hreflang ja/en の言語切替、gtag も英語ページに出力
- 一時的な `index.en.md` を置いたビルドで、英語ページが `/en/` に出て日本語側は不変であることを確認
- 一時的な `basics.en.md` で、コメントのみ翻訳 → 成功、クエリを書き換え → 差分を指摘して失敗、原文の無い `.en.md` → 失敗、
  known-failures を持つページの `.en.md` → 日本語のエントリを共有して成功、を確認
- actionlint、`cargo fmt --check`、`cargo clippy -p maharit-query --all-targets -D warnings`

### 補足
- 英語サイトでフォールバック表示される日本語ページも `<html lang="en">` になる（プラグインの仕様）。全ページ翻訳後は解消する

## 完了条件
- `mkdocs build --strict` が通り、`/en/` 配下に英語サイト（未翻訳ページは日本語へフォールバック）が生成される
- ヘッダーの言語切替で日本語・英語を行き来できる
- `cargo test -p maharit-query --test docs_cypher_examples` が英語ファイル込みで通る
- 用語集と CLAUDE.md の運用ルールがある

## 後続タスク
- 137: README の英語化
- 138: 英語翻訳 第 1 弾（入口ページ）
- 139: 英語翻訳 第 2 弾（Cypher / 関数 / インデックス）
- 140: 英語翻訳 第 3 弾（運用 / 高度なトピック / API / アルゴリズム / 可視化）
- 141: 翻訳の鮮度チェックを CI に追加
