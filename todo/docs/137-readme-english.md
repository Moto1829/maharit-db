# タスク 137: README の英語化

方針（2026-11-05 決定）: GitHub のトップに出す `README.md` を英語にし、日本語は `README.ja.md` に移す。

## 作業内容
- [ ] 現在の `README.md` を `git mv README.md README.ja.md` で移す（履歴を残す）
- [ ] 英語の `README.md` を新規作成する（構成は日本語版と揃える）
- [ ] 両ファイルの先頭に言語リンクを置く: `English | [日本語](README.ja.md)` / `[English](README.md) | 日本語`
- [ ] 翻訳のついでに、日本語版と英語版の両方で古くなっている記述を直す
  - 「mdBook 形式」→ 実際は MkDocs Material。リンク先を GitHub Pages（https://moto1829.github.io/maharit-db/ と `/en/`）にする
  - Rust バッジの `1.75+` → `rust-toolchain.toml` の版に合わせる
  - clone URL `github.com/suzukishimei/maharit-db` → `github.com/Moto1829/maharit-db`（`Cargo.toml` の `repository` も同様に誤っているので合わせて直す）
  - インストール手順に `install.sh` / `install.ps1`（GitHub Release のバイナリ）を追加する
  - 「BM25 全文検索（日本語対応）」は `japanese` feature を有効にしてビルドした場合のみである旨を書く
- [ ] `release.yml` のリリースノートに含まれるインストール手順の文言と矛盾しないことを確認する

## 完了条件
- GitHub のトップに英語 README が表示され、日本語版へ 1 クリックで行ける
- 両言語で記述内容（機能一覧・手順）が一致している
