# Create indexes

Source: https://neo4j.com/docs/cypher-manual/current/indexes/search-performance-indexes/create-indexes/

## 基本
- 作成コマンド: `CREATE [index_type] INDEX [index_name] ...`
- index_type を省略すると Range。
- `IF NOT EXISTS` で冪等。
- 名前はインデックス/制約全体で一意。
- 作成直後は `POPULATING`。`SHOW INDEXES` で `state` を確認。
- 作成には CREATE INDEX 権限が必要。

## Range index
- コマンド: `CREATE INDEX ... FOR (n:Label) ON (n.prop[, ...])`
- 単一/複合（複合は複数プロパティ）。
- 対応述語:
  - 等価 `n.prop = value`
  - リスト `n.prop IN list`
  - 存在 `n.prop IS NOT NULL`
  - 範囲 `n.prop > value`
  - 前方一致 `STARTS WITH`
- 構成オプションなし。

## Text index
- コマンド: `CREATE TEXT INDEX ... FOR (n:Label) ON (n.prop)`
- `STRING` 述語に限定。
- 対応述語:
  - 等価 `n.prop = 'str'`
  - リスト `n.prop IN ['a', 'b']`
  - `STARTS WITH` / `ENDS WITH` / `CONTAINS`
- トリグラム（3 文字）で索引化。`CONTAINS`/`ENDS WITH` に強い。
- 構成オプションなし。
- 近似/類似は Full-text index を利用。

## Point index
- コマンド: `CREATE POINT INDEX ... FOR (n:Label) ON (n.prop)`
- `POINT` 述語に限定。
- 対応述語:
  - `n.prop = point({...})`
  - `point.withinBBox(n.prop, lowerLeft, upperRight)`
  - `point.distance(n.prop, center) <= distance`
- `OPTIONS { indexConfig: { ... } }` で空間範囲を制限可能。
  - `spatial.cartesian.min/max`（2D）
  - `spatial.cartesian-3d.min/max`（3D）
  - `spatial.wgs-84.min/max`（2D）
  - `spatial.wgs-84-3d.min/max`（3D）

## Token lookup index
- コマンド: `CREATE LOOKUP INDEX ... FOR (n) ON EACH labels(n)`
- `CREATE LOOKUP INDEX ... FOR ()-[r]-() ON EACH type(r)`
- 既定で 2 つ（ラベル/タイプ）が存在。各種 1 つのみ作成可能。
- 対応述語: ラベル/タイプの一致のみ。
- 解除すると大幅に性能が低下する可能性。

## 競合時の挙動
- 同一スキーマ・同一種別: 既存インデックスがあれば作成失敗。
- 同名: 既存インデックス/制約と名前が競合すると失敗。
- 制約が同スキーマにある場合（特に range）: インデックス作成に失敗。
- `IF NOT EXISTS` で通知のみ（何もしない）。
