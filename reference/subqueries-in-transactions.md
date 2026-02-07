# CALL subqueries in transactions

Source: https://neo4j.com/docs/cypher-manual/current/subqueries/subqueries-in-transactions/

## 概要
- `CALL { ... } IN TRANSACTIONS` でバッチ実行。
- 大量更新/インポート/削除に適用。
- 内部トランザクションは既定で1000行バッチ。

## 構文
- `IN TRANSACTIONS [OF n ROWS]`
- `IN CONCURRENT TRANSACTIONS` で並列実行（非決定的な順序）。
- `REPORT STATUS AS var` で状態をMAPとして出力。

## エラー処理
- `ON ERROR CONTINUE` / `BREAK` / `FAIL` / `RETRY [FOR x SECONDS] [THEN ...]`
- 失敗したバッチはロールバック、成功したバッチは保持。

## 制限
- ネストした `IN TRANSACTIONS` は不可。
- `UNION` 内での利用不可。
- 書き込み句の後に置く場合制限あり。

## 注意
- `IN CONCURRENT TRANSACTIONS` はスロット実行のみ。
- `:auto` が必要な環境あり（Browser）。
