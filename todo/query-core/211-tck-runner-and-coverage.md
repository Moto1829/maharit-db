# タスク 211: TCK の実行器を直し、取り込む範囲を広げる

## 背景（2026-11-21）
task181 で取り込んだ実行器（`crates/maharit-query/tests/tck.rs`）は最小限で、次の点でエンジンでなく実行器の都合で落ちているシナリオがある。

- 期待値の文字列のエスケープ（Gherkin の表の `\\`、文字列の `\n`）を読んでいない: `literals/Literals6 [5]`、`string/String8 [5]` `String9 [5]` `String10 [5]`
- エラーの種類（`SyntaxError` / `TypeError` / `ArgumentError`）と時期（解析時・実行時）を区別していない。どれでもエラーなら通る
- 副作用（`the side effects should be`、作成・削除の件数）を確かめていない
- 取り込んでいるのは `expressions/` の 10 領域だけ。`clauses/`（MATCH・CREATE・MERGE・WITH など）と `expressions/` の残り（aggregation・map・quantifier・typeConversion など）が無い

## 作業
- [ ] 期待値のエスケープを正しく読む（落ちていた分は一覧から外れるはず）
- [ ] エラーの種類と時期を、`ExecuteError` / `ParseError` の種類に対応づけて確かめる（対応表を作る）
- [ ] 副作用の件数を、書き込み文の結果の件数と比べる
- [ ] `clauses/` と `expressions/` の残りを取り込み、未対応一覧を作り直す。通過件数を docs の「標準への準拠」に反映する
- [ ] 取り込みで見つかったエンジンの不具合は、領域ごとにタスクにする
