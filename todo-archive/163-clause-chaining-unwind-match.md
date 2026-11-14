# タスク 163: UNWIND / MATCH / WITH を自由な順序でつなげられるようにする

## 現状（2026-11-12、task124 で確認）
文は決まった形（`MATCH … WITH … RETURN`、`UNWIND … CREATE … RETURN` など）ごとに実装されていて、
次の組み合わせがパースできない。

- `MATCH (a:Article) UNWIND a.tags AS tag RETURN a.title, tag`（MATCH の後の UNWIND）
- `UNWIND [1, 2, 3] AS id MATCH (n:Product) WHERE n.id = id RETURN n`（UNWIND の後の MATCH）
- `UNWIND [[1, 2], [3, 4]] AS sub UNWIND sub AS num RETURN num`（UNWIND の連続）
- `MATCH … WITH … UNWIND stats AS stat WITH stat, avg WHERE … RETURN …`（WITH の後の UNWIND）
- トップレベルの `MATCH … MATCH …`、`WITH 1 AS x RETURN x`、`OPTIONAL MATCH` で始まる文
- `MATCH … CREATE … SET` など、書き込みの後の SET

docs/cypher/unwind-with-union.md・functions/aggregation.md・functions/string.md の例が
`docs_cypher_known_failures.txt` に残っている。

## 方針（案）
- 読み取りの文を「句の列」（MATCH / OPTIONAL MATCH / UNWIND / WITH / WHERE）として持ち、
  バインディングの列に順に適用するパイプラインにする（`QuerySegment` の一般化）
- まず読み取りの組み合わせから。書き込み句の自由な連結はその後
- planner（EXPLAIN）・is_read_only・コーディネーター（coordinator_plan）の対応も必要

## 結果（2026-11-14）
- `QuerySegment.unwinds`（先頭の UNWIND 句）を追加。セグメントは UNWIND → MATCH/OPTIONAL MATCH（任意個）→ WHERE → WITH の順に適用し、
  MATCH の後の UNWIND・WITH … WHERE の後の MATCH で次のセグメントに移る
- 文の先頭に UNWIND（後ろに MATCH / UNWIND / WITH が続く場合）・OPTIONAL MATCH・WITH を書ける。トップレベルの `MATCH … MATCH` も可
- UNWIND は null で 0 行、リスト以外は 1 行。ノード・リレーションシップの値はそのまま束縛するので後続のパターンで使える
- 後ろに CREATE / SET / REMOVE / FOREACH を続けるのは既存の文の形で動く（`UNWIND … MATCH … SET` など）
- EXPLAIN に `Unwind` を表示
- docs の known-failures 5 件が解消（task124 も完了）
- 書き込み句の後の連結は query-clauses/171 に分けた
