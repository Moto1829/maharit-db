# タスク 132: lindera（japanese feature）を最新系へ上げる

## 概要（2026-11-03）

`maharit-core` の `japanese` feature が使う lindera 2.2 は古く（最新は 6.x）、配下に次の警告がある。

- メンテ終了: `encoding`（RUSTSEC-2021-0153）、`unsafe-libyaml`（RUSTSEC-2024-0449）
- unsound: `memmap2`、`anyhow`
- 脆弱性（`cargo update` で解消できる。task 128）: `rkyv`、`tar`、`quinn-proto`

また lindera-ipadic はビルド時に `reqwest` で辞書をダウンロードする（ビルドがネットワークに依存する）。

## 作業
- lindera 6.x に上げ、`fulltext.rs` の日本語トークナイザの API 変更に追従する
- ビルド時ダウンロードの有無と、オフラインでビルドできるかを確認する
- `cargo test --workspace --features maharit-core/japanese` で日本語全文検索のテストを通す
- 既定ビルドには含まれない feature なので、優先度は task 128〜130 より低い
