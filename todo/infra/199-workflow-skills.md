# タスク 199: 定型作業をスキルにする

## 背景
CLAUDE.md の「新ステートメント追加の典型手順」は 1 行しかない。
実際には、実装のほかに docs・適合表・既知の未対応一覧の更新が要り、AI はここを漏らしやすい（メモリにも「KNOWN_UNSUPPORTED の更新も忘れずに」とある）。

手順をスキル（`.claude/skills/<名前>/SKILL.md`）にすれば、該当する作業のときだけ読み込まれ、CLAUDE.md も短くできる。

## 作業内容
- [ ] `add-cypher-function`: 組み込み関数の追加
  - 実装場所、引数の検査と null の扱いの決まり、エラーの返し方
  - 単体テスト → `conformance_tests.rs` → `docs/functions/` → `docs_cypher_known_failures.txt` から該当行を消す
- [ ] `add-cypher-clause`: 句・文の追加
  - `ast.rs` → `lexer.rs` → `parser.rs` → `executor` → `planner.rs` → `is_read_only` の分類 → docs
  - キーワードを増やすと既存の識別子・ラベルが壊れないかの確認（bug/175 の類）
- [ ] `fix-bug`: `todo/bug/` のタスクの進め方
  - 再現テストを先に書いて落ちることを確かめる → 修正 → `bash scripts/check.sh` → タスクを `todo-archive/` へ移す → 1 コミットで push
  - 互換を壊す修正に `!` を付ける判断基準（CLAUDE.md を参照）
- [ ] `release`: CLAUDE.md の「リリース手順」をスキルへ移し、CLAUDE.md には 1 行の参照だけ残す
- [ ] 各スキルの `description` に、いつ使うかを具体的に書く（「関数を追加するとき」など。ここが曖昧だと読み込まれない）
- [ ] 実際のタスクを 1 件ずつスキル経由で進め、漏れた手順をスキルに足す

## 完了条件
- 関数を 1 つ追加する作業で、docs・適合表・既知の未対応一覧の更新が指示なしで行われる
- CLAUDE.md からリリース手順の詳細が消え、行数が減っている
