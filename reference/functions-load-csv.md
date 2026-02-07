# LOAD CSV functions

Source: https://neo4j.com/docs/cypher-manual/current/functions/load-csv/

## 関数
- `linenumber()`：現在の行番号（ヘッダは1）。
- `file()`：LOAD CSVのファイル絶対パス。

## 備考
- LOAD CSV コンテキスト以外では `null`。
- 複数LOAD CSV時は直近のものが対象。
