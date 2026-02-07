# Drop indexes

Source: https://neo4j.com/docs/cypher-manual/current/indexes/search-performance-indexes/drop-indexes/

## 基本
- コマンド: `DROP INDEX index_name [IF EXISTS]`
- `IF EXISTS` で冪等。
- 権限: DROP INDEX。
- 制約が所有するインデックスは直接削除不可。制約を削除すると関連インデックスも削除。
- 別バージョンで作成されたインデックスも削除可能。

## 例外/挙動
- 存在しないインデックスを削除するとエラー（`IF EXISTS` なら通知のみ）。
- 制約の背後インデックスを削除するとエラー。
