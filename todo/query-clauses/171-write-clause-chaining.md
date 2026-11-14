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
