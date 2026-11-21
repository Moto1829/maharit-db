# タスク 196: 検証を 1 コマンドで回せるようにする

## 背景
コミット前に回すべき検査が複数あり、手順は CLAUDE.md とメモリに散らばっている。

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`（`--workspace` を忘れるとルートパッケージしか検査されない）
- `cargo test --workspace`
- `cargo test -p maharit-query --test docs_cypher_examples`
- `python3 scripts/check_docs_i18n.py`

AI は毎回これを組み立てており、`--workspace` の付け忘れや、失敗しても push まで進む（`;` でつないだ）事故が過去に起きている。
注意書きで防ぐのではなく、コマンド 1 本にして間違えようがない形にする。
タスク 197（executor の分割）を安全に進める前提にもなる。

## 作業内容
- [x] `scripts/check.sh` を作る（CI の `test.yml` と同じ検査を同じ順で実行し、最初の失敗で止まる）
- [x] `--quick` を付ける: `git diff --name-only origin/main...` から変更のあったクレートを求め、そのクレートと依存元だけ clippy / test を回す。docs だけの変更なら Cypher 例の検査と鮮度チェックだけにする
- [x] 失敗時は、どの段で落ちたかを最後に 1 行で出す（AI が長い出力の末尾だけ読んでも分かるように）
- [x] CI の `test.yml` も可能な範囲でこのスクリプトを呼ぶ形にし、手元と CI の検査内容がずれないようにする
- [x] CLAUDE.md の「ビルド・テスト・実行コマンド」節の先頭に「コミット前は `bash scripts/check.sh`」と書く

## 完了条件
- `bash scripts/check.sh` が通れば CI の fmt / clippy / test も通る
- `bash scripts/check.sh --quick` が、1 クレートだけの変更で全体実行より明らかに短い時間で終わる
