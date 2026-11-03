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

## 対応内容（2026-11-03）
- lindera 2.2 → 6.2。lindera 6 で `Tokenizer` が無くなったため `Segmenter::segment` を直接使う形に移行
- 性能上の不具合も修正: キャッシュしない方の `tokenize_japanese` は呼ばれるたびに辞書を読み込み直していた
  （検索クエリのトークナイズのたびに辞書ロード）。スレッドローカルにキャッシュした `Segmenter` に一本化し、
  `tokenize_cached` は不要になったので削除（japanese のテスト時間 2.20 秒 → 0.06 秒）
- 分かち書きの互換性: 代表 5 文の結果を lindera 2.2 で記録し、6.2 で完全に一致することを確認。
  記録した結果を回帰テスト `japanese_segmentation_tests` として追加（索引と検索で版が違っても一致させるため）
- `cargo audit`: 脆弱性・警告とも 0 件（encoding / unsafe-libyaml のメンテ終了警告が消えた）。
  reqwest も依存から消えた（ビルド時の辞書ダウンロードは ureq。取得先 lindera.dev、MD5 検証）
- **バイナリサイズ**: lindera 6 は辞書を無圧縮で埋め込むため、`japanese` 付きリリースが 18.09MB → 48.59MB。
  既定ビルドは 2.70MB で変化なし。ユーザー判断で 6.2 を採用。小さくしたい場合は外部辞書ファイルのロードを別タスクで検討
- オフラインビルド: `LINDERA_BUILD_DICTIONARY_CACHE_DIR` を指定すると辞書をキャッシュでき、2 回目以降はダウンロード不要。
  docs/functions/fulltext.md と maharit-core/Cargo.toml のコメントに記載
- 検証: fmt・clippy（japanese 有無の両方）クリーン、`cargo test --workspace` 全通過（既定 1157 件 / japanese 1169 件）、
  `bash scripts/run_e2e_local.sh` 全スイート通過

## ステータス
完了
