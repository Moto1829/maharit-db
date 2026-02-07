# Show indexes

Source: https://neo4j.com/docs/cypher-manual/current/indexes/search-performance-indexes/list-indexes/

## 基本
- `SHOW INDEXES` で一覧表示。
- `SHOW INDEXES YIELD *` で全列表示。
- 権限: SHOW INDEX。

## 絞り込み
- `SHOW RANGE INDEXES` / `SHOW TEXT INDEXES` / `SHOW POINT INDEXES` / `SHOW LOOKUP INDEXES` / `SHOW FULLTEXT INDEXES` / `SHOW VECTOR INDEXES`。
- `WHERE` 句でフィルタ可能（例: `owningConstraint IS NULL`）。
- `YIELD` と `RETURN` を組み合わせて特定列のみ返す。

## 主要カラム（YIELD *）
- `id`: インデックス ID。
- `name`: インデックス名（指定 or 自動生成）。
- `state`: `ONLINE` / `POPULATING` など。
- `populationPercent`: 構築進捗。
- `type`: `FULLTEXT`/`LOOKUP`/`POINT`/`RANGE`/`TEXT`/`VECTOR`。
- `entityType`: `NODE` / `RELATIONSHIP`。
- `labelsOrTypes`: 対象ラベル/関係タイプ。
- `properties`: 対象プロパティ（vector は先頭が vector、その後はフィルタ用追加プロパティ）。
- `indexProvider`: プロバイダ名。
- `owningConstraint`: 制約の背後インデックスの場合、その制約名。
- `lastRead`: 最終読取時刻（未使用なら null）。
- `readCount`: 読取回数。
- `trackedSince`: 統計開始時刻。
- `options`: `OPTIONS` 由来の構成。
- `failureMessage`: 失敗理由。
- `createStatement`: 作成に使われた文（互換性問題で null の場合あり）。

## メモ
- `YIELD` は `RETURN` を使う場合に必須。
