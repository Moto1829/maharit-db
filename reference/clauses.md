# Clauses

Source: https://neo4j.com/docs/cypher-manual/current/clauses/

## データフローの前提
- Cypherクエリ内のデータフローは**順序を持たないマップ（キー/値）の集合**として扱われる。
- 句が順番にこの集合を絞り込み/拡張していく。

## Reading clauses（読み取り）
- `FILTER`
  - クエリにフィルタを追加。
  - Cypher 25で導入（Neo4j 2025.06）。
- `MATCH`
  - グラフ内のパターンを検索する。
- `OPTIONAL MATCH`
  - パターンが存在しない場合に`null`を返す探索。

## Projecting clauses（投影）
- `FINISH`
  - 結果を返さず副作用のみ実行。
- `LET`
  - 式結果を変数に束縛（Cypher 25で導入）。
- `RETURN [AS]`
  - 結果セットに含める式を定義。
- `UNWIND [AS]`
  - リストを行へ展開。
- `WITH [AS]`
  - クエリパート間で結果をパイプし、スコープを制御。

## Reading sub-clauses（読み取りのサブ句）
- `WHERE`
  - `MATCH` / `OPTIONAL MATCH` の制約追加、または `WITH` 結果のフィルタ。
- `ORDER BY [ASC|DESC]`
  - `RETURN`/`WITH`の出力ソート。
- `SKIP` / `OFFSET`
  - 返却行の開始位置を指定。
- `LIMIT`
  - 返却行数の上限。

## Writing clauses（書き込み）
- `CREATE`
  - ノード/リレーションシップ作成。
- `DELETE`
  - ノード/関係/パス削除（ノード削除時は関係も明示的に削除が必要）。
- `DETACH DELETE`
  - ノード削除時に関連する関係を自動削除。
- `SET`
  - ラベル/プロパティを更新。
- `REMOVE`
  - ラベル/プロパティを除去。
- `FOREACH`
  - リストやパス構成要素に対して更新処理を行う。

## Reading/Writing clauses（読み書き）
- `MERGE`
  - パターンが存在することを保証。存在しなければ作成。
- `ON CREATE`
  - `MERGE`で作成時に実行する処理。
- `ON MATCH`
  - `MERGE`で既存時に実行する処理。
- `CALL [YIELD]`
  - プロシージャを呼び出し、結果を返す。

## Subquery clauses
- `CALL { ... }`
  - サブクエリを評価。
- `CALL { ... } IN TRANSACTIONS`
  - サブクエリを複数トランザクションに分割して評価（大量更新/インポート向け）。

## Set operations
- `UNION`
  - 重複を除外して結果結合。
- `UNION ALL`
  - 重複を保持して結果結合。

## Multiple graphs
- `USE`
  - クエリ/クエリパートの実行対象グラフを指定。

## Importing data
- `LOAD CSV`
  - CSVファイルの読み込み。
- `CALL { ... } IN TRANSACTIONS`
  - `LOAD CSV`での大量処理時の分割実行。

## Listing functions and procedures
- `SHOW FUNCTIONS`
  - 利用可能な関数一覧。
- `SHOW PROCEDURES`
  - 利用可能なプロシージャ一覧。

## Configuration commands
- `SHOW SETTINGS`
  - 設定一覧。

## Transaction commands
- `SHOW TRANSACTIONS`
  - トランザクション一覧。
- `TERMINATE TRANSACTIONS`
  - 指定トランザクションの終了。

## Reading hints（ヒント）
- `USING INDEX`
  - インデックス使用を指定。
- `USING INDEX SEEK`
  - インデックスシーク使用を指定。
- `USING SCAN`
  - ラベルスキャンを強制。
- `USING JOIN`
  - ジョイン方法の指定。

## Index and constraint clauses
- `CREATE | SHOW | DROP INDEX`
  - インデックスの作成/表示/削除。
- `CREATE | SHOW | DROP CONSTRAINT`
  - 制約の作成/表示/削除。

## Administration clauses
- DB/エイリアス/サーバー/RBACなどの管理コマンドはOperations Manualで定義される。
- 参照: https://neo4j.com/docs/operations-manual/current/database-administration/
- 参照: https://neo4j.com/docs/operations-manual/current/authentication-authorization/
- 参照: https://neo4j.com/docs/operations-manual/current/clustering/
