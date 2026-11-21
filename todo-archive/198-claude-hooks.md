# タスク 198: Claude Code のフックで「落とし穴」を自動で防ぐ

## 背景
メモリには「忘れると事故になる注意」がいくつも溜まっている。

- リポジトリ内で REPL を動かすと、追跡対象の `.maharit_history` が変わる（確認後に戻す）
- 共有の作業ディレクトリでは、他のセッションがブランチを切り替えていることがある（commit 前に HEAD を確認）
- テストを通ってからコミットする

注意書きは読み飛ばされることがある。フックなら毎回必ず実行される。
現在、チェックイン済みの `.claude/settings.json` は無い（`settings.local.json` だけ）。

## 作業内容
- [x] `.claude/settings.json` を作り、リポジトリに入れる
- [x] PostToolUse（Edit / Write、対象が `.rs`）: そのファイルに `rustfmt --edition 2024` をかける
- [x] Stop: `.maharit_history` に差分があれば知らせる（自動で戻すか、知らせるだけかを決める）
- [x] PreToolUse（Bash、`git commit` を含むコマンド）: 現在のブランチ名を表示する。main への直接コミットを止めるかどうかは運用に合わせて決める（現状は main へ直接 push する運用）
- [x] フックのスクリプトは `.claude/hooks/` に置き、失敗しても作業を止めないもの（整形）と止めるもの（検査）を分ける
- [x] フックで置き換えられたメモリ（`repl-history-tracked.md`、`shared-worktree-branch-check.md` など）を削除または短くする

## 補足
- `.maharit_history` を追跡対象から外して `.gitignore` に入れられるなら、その方が単純。追跡している理由を先に確認する
- `settings.local.json` には heredoc を丸ごと含む一度きりの許可が大量に溜まっている。`/fewer-permission-prompts` で整理し、共通の許可（`cargo` / `git` の読み取り系）は `settings.json` 側へ移す

## 完了条件
- `.rs` を編集した直後に `cargo fmt --all -- --check` が通る
- REPL を動かしたセッションの終わりに `.maharit_history` の差分が知らされる

## 結果（2026-11-21）
- `.maharit_history` は過去に誤ってコミットされたまま追跡されていただけだったので、追跡をやめて `.gitignore` に入れた。Stop フックは作らない
- PostToolUse（Edit / Write / MultiEdit）: `.claude/hooks/rustfmt-edited.sh` が `.rs` に `rustfmt --edition 2024` をかける（失敗しても止めない）
- PreToolUse（Bash）: `.claude/hooks/show-branch-on-commit.sh` が `git commit` の前にコミット先のブランチを表示する（止めない。main へ直接 push する運用のため）
- メモリ `repl-history-tracked` を削除し、`shared-worktree-branch-check` をフック前提に短くした
- `settings.local.json` の整理（`/fewer-permission-prompts`）は各自の手元の設定なので、このタスクでは行わない
