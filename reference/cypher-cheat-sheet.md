# Cypher Cheat Sheet

Source: https://neo4j.com/docs/cypher-cheat-sheet/25/all/

## 読み取りクエリ構造（Read Query Structure）
- 典型構造:
	- `USE` → `MATCH`/`OPTIONAL MATCH` → `WITH` → `RETURN`（各句に`WHERE`/`ORDER BY`/`SKIP`/`LIMIT`を付与可能）
- キーワードは**大文字小文字を区別しない**。変数名は**区別する**。

## 書き込みクエリ構造
- **Write-Only**: `USE` → `CREATE` → `MERGE` → `WITH` → `SET` → `DELETE` → `REMOVE` → `RETURN`
- **Read-Write**: 読み取り句の後に `CREATE`/`MERGE`/`SET`/`DELETE`/`REMOVE` を続ける。

## 主要句（抜粋）
- `MATCH`: パターン検索。
	- 動的ラベル/型の参照が可能（`$()`式）。
- `OPTIONAL MATCH`: 存在しない場合は`null`を返す。
- `WHERE`: フィルタ。パターン内/可変長パターン/パターン内の`WHERE`にも対応。
- `FILTER`: `WHERE`と同様のフィルタだが**独立した句**として使える（`MATCH`/`WITH`のサブ句ではない）。
- `RETURN`: 結果列定義。`DISTINCT`、`ORDER BY`、`SKIP`、`LIMIT`、`WHERE`と併用可能。
- `WITH`: 中間結果の受け渡し。`*`で全変数維持、`DISTINCT`や集約、`ORDER BY`/`LIMIT`も可能。
- `LET`: 式結果を変数に束縛（式チェーンに適する）。
- `CREATE`: ノード/関係作成。動的ラベル/型も可。
- `MERGE`: パターンの存在保証。`ON CREATE`/`ON MATCH`に対応。
- `SET`: プロパティ更新/追加、ラベル追加。`=`は全置換、`+=`は追加/更新。
- `DELETE` / `DETACH DELETE`: 関係/ノード削除。`DETACH`は関係も含めて削除。
- `REMOVE`: ラベル/プロパティ削除。全プロパティ削除は`SET n = {}`で行う。
- `UNWIND`: リストを行へ展開。
- `LOAD CSV`: CSVインポート、`WITH HEADERS`、`FIELDTERMINATOR`、`IN TRANSACTIONS`に対応。
- `CALL`: プロシージャ呼び出し。`YIELD`必須。`OPTIONAL CALL`可。
- `FINISH`: 結果を返さず副作用のみ実行。
- `FOREACH`: リスト要素に対して更新処理。
- `USE`: 実行対象グラフ/DBを指定。
- `SHOW FUNCTIONS/PROCEDURES/SETTINGS/TRANSACTIONS`: 一覧系。
- `TERMINATE TRANSACTIONS`: トランザクション停止。

## クエリ合成
- `UNION` / `UNION ALL` で結果結合。列名/型は一致必須。
- `WHEN ... THEN ... ELSE ...`: 条件分岐クエリ。
- `NEXT`: 直列合成。前段の結果テーブルを次段へ渡す。

## パターン
- **固定長パターン**: 単純な関係連鎖。
- **可変長/量指定**: `{1,3}`や`+`/`*`。
- **最短路**: `SHORTEST k`、`ALL SHORTEST`、`SHORTEST k GROUPS`。
- **ANY**: 到達可能性の意図を表現（`SHORTEST 1`相当）。
- **Match mode**:
	- 既定は`DIFFERENT RELATIONSHIPS`（関係の一意性要求）。
	- `REPEATABLE ELEMENTS`は関係再訪可。ただし上限付きが必須。

## 述語（Predicates）
- **論理**: `AND`, `OR`, `XOR`, `NOT`。
- **比較**: `=`, `<>`, `<`, `<=`, `>`, `>=`, `IS NULL`, `IS NOT NULL`。
- **リスト**: `IN`。
- **文字列**: `STARTS WITH`, `ENDS WITH`, `CONTAINS`, `=~`（正規表現）、`IS NORMALIZED`。
- **型検査**: `IS :: TYPE` / `IS NOT :: TYPE`、`NOT NULL`付与可。

## 式（Expressions）
- **CASE**: simple/extended/genericをサポート。
- **ラベル式**: `|`（OR）、`!`（NOT）。
- **リスト式**: `[]`アクセス、`..`スライス、`||`/`+`結合、内包表記、パターン内包。
- **ノード/関係演算子**: `.`/`[]`でプロパティアクセス。
- **数学演算子**: `+`, `-`, `*`, `/`, `%`, `^`。
- **文字列結合**: `||`（GQL準拠）と`+`。
- **時間演算子**: temporal + duration。
- **Map式**: `.`/`[]`アクセス、map投影（`{.a, .b}`、`{.*}`など）。

## 関数カテゴリ
- **集約**: `avg`, `collect`, `count`, `min`, `max`, `percentileCont/Disc`, `stDev`, `stDevP`, `sum`。
- **DB/Graph**: `db.nameFromElementId`, `graph.names`, `graph.byName`, `graph.byElementId`, `graph.propertiesByName`。
- **リスト**: `keys`, `labels`, `nodes`, `relationships`, `range`, `reduce`, `reverse`, `tail`, `toBooleanList`, `toFloatList`, `toIntegerList`, `toStringList` など。
- **数学**: `abs`, `ceil`, `floor`, `rand`, `round`, `sign`, `log`, `log10`, `sqrt`, `sin/cos/tan`, `sinh/cosh/tanh`, `degrees/radians` 等。
- **述語関数**: `all`, `any`, `none`, `single`, `exists`, `isEmpty`, `allReduce`。
- **スカラー**: `char_length/character_length`, `coalesce`, `elementId`, `id`(非推奨), `length`, `properties`, `randomUUID`, `size`, `timestamp`, `toBoolean/Float/Integer`, `valueType` 等。
- **文字列**: `trim`/`ltrim`/`rtrim`/`btrim`, `split`, `substring`, `replace`, `normalize`, `toLower/upper` 等。
- **空間**: `point`, `point.distance`, `point.withinBBox`。
- **時間**: `date`, `time`, `datetime`, `localtime`, `localdatetime`, `duration` と `*.truncate`、`duration.between` 等。
- **format**: `format()`で時刻/期間を文字列化。
- **ベクトル**: `vector.similarity.*` など。

## スキーマ（インデックス/制約）
- **インデックス**: `CREATE [RANGE|TEXT|POINT|LOOKUP|FULLTEXT|VECTOR] INDEX`。
	- `SHOW INDEXES`, `DROP INDEX`。
	- `USING INDEX`でヒント指定。
- **フルテキスト**: `CREATE FULLTEXT INDEX`、`db.index.fulltext.queryNodes/Relationships`。
- **ベクトル**: `CREATE VECTOR INDEX`、`db.index.vector.queryNodes/Relationships`。
- **制約**: `SHOW CONSTRAINTS`、`CREATE CONSTRAINT`（一意、存在、型、キー）。

## パフォーマンス指針
- パラメータ化でクエリキャッシュを活用。
- 可変長パターンには上限を付ける。
- 必要なデータのみ返す。
- `EXPLAIN`/`PROFILE`でプラン分析。

## 管理/アクセス制御（抜粋）
- **DB管理**: `SHOW DATABASES`, `CREATE/DROP/ALTER DATABASE`, `SHOW ALIASES`, `CREATE/ALTER/DROP ALIAS`。
- **権限管理**: `SHOW USERS/ROLES/PRIVILEGES`、`GRANT/DENY/REVOKE`。
- **ON GRAPH/ON DATABASE/ON DBMS** の各種権限制御が細分化。
