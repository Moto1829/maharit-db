# タスク 171: 書き込み句の後に句をつなげられるようにする

## 現状（2026-11-14、task163 の残り）
task163 で読み取りの句（UNWIND / MATCH / OPTIONAL MATCH / WITH）は自由な順序でつなげられるようになった
（その後の CREATE / SET / REMOVE / FOREACH も可）。書き込み句の後には続けられない。

- `MATCH (a) CREATE (b) SET b.x = 1`（CREATE の後の SET）
- `CREATE (a) CREATE (b)`、`CREATE (a) WITH a MATCH (b) …`（CREATE の後の句）
- `UNWIND … MATCH … DETACH DELETE` / `UNWIND … MERGE`（UNWIND を含む DELETE / MERGE。今はパースエラーにしている）
- `MATCH (a) WITH a MERGE …` は WITH が無視される（MERGE の文が先頭セグメントの MATCH しか持たない）

## 方針（案）
- 文を「句の列」として持ち、書き込み句もバインディングの列に順に適用する
- 書き込み後の読み取りは書き込みの結果を見えるようにする（Cypher の意味）
- is_read_only・EXPLAIN・コーディネーター（書き込みは拒否）の対応

## 結果（2026-11-14）
- `Statement::Pipeline`（`PipelineStatement { clauses: Vec<PipelineClause>, return_clause }`）を追加。句を書かれた順に行の列へ適用する
- パーサーは従来の決まった形の文として読み、書き込み句の後に句が残っていたり読めなかったりしたときだけ、先頭から句の列として読み直す
  （従来の文の意味・結果の形は変わらない。読み直しにも失敗したら元のエラーを返す）
- RETURN を省略した句の列は created_nodes / created_edges / deleted_nodes / deleted_edges / properties_set を返す
- `MATCH … WITH … MERGE` は WITH を無視していたので、句の列で WITH を効かせるようにした
- EXPLAIN は句ごとに Create / Merge / SetProperties / Remove / Delete / Foreach を表示。コーディネーターは読み取りだけの句の列を拒否
- docs/cypher/basics に「句をつなげる」を追加
