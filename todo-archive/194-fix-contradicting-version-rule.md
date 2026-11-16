# タスク 194: CLAUDE.md と矛盾するバージョン規則ファイルを直す

AI のコード生成の質を上げるための一連のタスク（194〜201）の 1 つ目。数分で終わる。

## 背景
`.claude/skills/rules/version.md` に「コードを変更した実装した場合は必ずバージョンをインクリメントすること」とある。
CLAUDE.md は「バージョンはリリース時にだけ上げる。普段のコミットで `Cargo.toml` の version を触らない」と定めており、正反対になっている。
AI がこのファイルを読むと、普段のコミットで version を書き換えるおそれがある。

また、このディレクトリには `SKILL.md` が無いので、スキルとしては読み込まれていない（検索で見つけたときだけ読まれる）。

## 作業内容
- [ ] `.claude/skills/rules/version.md` を削除する（内容は CLAUDE.md の「バージョニング・リリース」節が正本）
- [ ] `.claude/` 配下に、他にも CLAUDE.md と食い違う規則ファイルが無いか確認する（`.claude/agents/e2e-test-analyzer.md` と `.claude/agent-memory/` を含む）

## 完了条件
- リポジトリ内に「コード変更のたびに version を上げる」という記述が残っていない
