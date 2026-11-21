# タスク 201: workspace の lint を設定して機械的に弾く

## 背景
ルートの `Cargo.toml` に `[workspace.lints]` が無く、`clippy.toml` も無い。
現状の基準は clippy の既定 + `-D warnings` だけ。

AI の書いたコードの質は、レビューの指摘より lint の方が確実に揃う（書いた直後に自分で検出して直せる）。
特にこのプロジェクトは `unsafe`（`Executor` の生ポインタ、`unsafe impl Sync`）を含むので、理由の書かれていない `unsafe` を通さないようにしたい。

`executor.rs` の本体（テスト以外）の `unwrap()` は現在 2 か所だけで、状態は良い。これを維持する仕組みにする。

## 作業内容
- [x] ルートの `Cargo.toml` に `[workspace.lints.clippy]` / `[workspace.lints.rust]` を追加し、各クレートに `[lints] workspace = true` を入れる
- [x] 候補（1 つずつ有効にして、違反の件数を見てから決める）:
  - `clippy::undocumented_unsafe_blocks`: `unsafe` に `// SAFETY:` を必須にする
  - `clippy::unwrap_used` / `clippy::expect_used`: テスト以外で禁止（`clippy.toml` の `allow-unwrap-in-tests = true`）
  - `clippy::dbg_macro` / `clippy::todo` / `clippy::print_stdout`（REPL と CLI は個別に許可）
  - `rust::unsafe_op_in_unsafe_fn`
  - `clippy::too_many_lines` / `clippy::cognitive_complexity`: 閾値は現状の最大値を見て決める（タスク 197 の後）
- [x] 既存の違反は、直せるものは直し、残すものは理由を書いた `#[expect(...)]` を付ける（`#[allow]` ではなく `#[expect]` にすると、不要になったとき警告が出る）
- [x] 1 つの lint につき 1 コミットにする

## 完了条件
- `cargo clippy --workspace --all-targets -- -D warnings` が通る
- 理由の書かれていない `unsafe` ブロックを足すと clippy が失敗する

## 結果（2026-11-21）

違反の件数（テスト以外）を測って決めた。

| lint | 件数 | 決定 |
|---|---|---|
| `clippy::undocumented_unsafe_blocks` | 5 | 有効（workspace）。5 か所に `// SAFETY:` を書いた |
| `clippy::dbg_macro` / `todo` / `unimplemented` | 0 | 有効（workspace） |
| `rust::unsafe_op_in_unsafe_fn` | 0 | 有効（workspace） |
| `clippy::unwrap_used` | 126（query 8、core 17、storage 38、viz 37 など） | maharit-query だけ有効（`#![deny]`）。8 か所を直した。他のクレートは Mutex の poison や内部で保証した前提の `get_mut().unwrap()` が大半で、1 件ずつ見る必要がある |
| `clippy::expect_used` | 15 | 有効にしない（`expect` はメッセージで前提を書く手段として使っている） |
| `clippy::print_stdout` | 177 | 有効にしない（REPL・CLI・viz の出力が大半） |
| `clippy::too_many_lines` | 23 | 有効にしない（`evaluate_scalar_function` 約 1,000 行など。関数の分割を先にやる必要がある） |

確認: 理由の無い `unsafe` を足すと `cargo clippy` が `unsafe block missing a safety comment` で失敗する。
