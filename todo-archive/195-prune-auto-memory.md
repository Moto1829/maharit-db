# タスク 195: 自動メモリ（MEMORY.md）を整理する

## 背景
`~/.claude/projects/-Users-suzukishimei-Git-maharit-db/memory/MEMORY.md` は毎セッション全文が読み込まれる。
現在 115 行あり、大半が索引ではなく本文になっている。古い記述や CLAUDE.md と食い違う記述もある。

- 「Test Counts (latest)」: 古いテスト件数
- 「Completed Tasks」: 完了タスクの一覧（git 履歴と `todo-archive/` で分かる）
- 「Project Structure」「Key Architecture」「New Architecture Details」: CLAUDE.md と重複。クレート数が 7 個のまま（実際は 8 個）
- 「Versioning & Git Tagging Rules」: 「feat!+BREAKING CHANGE→MAJOR」とあるが、0.x の間は MINOR（CLAUDE.md が正本）

古い情報は AI の判断を誤らせ、重複は読み込むたびにコンテキストを消費する。

リポジトリの外にあるファイルなので、このタスクはコミットを伴わない（タスクファイルの archive 移動だけ）。

## 作業内容
- [ ] MEMORY.md を「1 行 1 メモリの索引」だけにする（目安 20 行以内）
- [ ] 上の 4 種類の節を削除する。CLAUDE.md に無くて今も正しい内容があれば、CLAUDE.md かクレート別の CLAUDE.md（タスク 200）へ移す
- [ ] 「Implementation Patterns」のうち今も有効なものは、1 事実 1 ファイルのメモリに分けるか CLAUDE.md へ移す
- [ ] 個別メモリファイル（`*-2026-07.md` など作業記録に近いもの）を見直し、今後の作業で行動が変わらないものは削除する
- [ ] `project_versioning.md` を CLAUDE.md と突き合わせ、重複なら削除する

## 完了条件
- MEMORY.md に索引行以外の本文が無い
- メモリの内容が CLAUDE.md と矛盾しない
