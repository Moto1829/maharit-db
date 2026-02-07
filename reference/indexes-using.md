# The impact of indexes on query performance

Source: https://neo4j.com/docs/cypher-manual/current/indexes/search-performance-indexes/using-indexes/

## 概要
- Search-performance indexes は `MATCH` の開始点最適化に使われる。
- 実行計画（`PROFILE`/`EXPLAIN`）で使用状況を確認可能。

## 種類別のポイント
### Token lookup indexes
- ラベル/タイプ条件のみ解決。
- DB 作成時に既定で存在し、削除は性能悪化の原因。

### Range indexes
- 等価/範囲/前方一致/存在など広範な述語を解決。
- Range index があると `NodeIndexSeek` が使われる。

### Text indexes
- `STRING` 述語（`CONTAINS`/`ENDS WITH`）で優先的に利用。
- `STARTS WITH` や等価は Range が優先される場合が多い。
- 8KB 超の `STRING` は Range の上限を超えるため Text が有利。
- `toString()` で型を明示すると Text index が使われやすい。

### Point indexes
- 距離検索やバウンディングボックスに最適。
- `OPTIONS.indexConfig` で空間範囲を限定可能。

## Composite indexes
- 複合は Range のみで利用可能。
- すべてのプロパティに対する述語が必要。
- 述語の評価効率はプロパティ順に依存。
- ルール:
  - 先頭プロパティから等価/リストを適用。
  - 範囲/前方一致は最大 1 つ。
  - 以降の述語は存在チェックに落ちる。
  - `CONTAINS`/`ENDS WITH` も存在チェック扱い。

## Range index-backed ORDER BY
- Range index の順序を利用し `Sort` を省略できることがある。
- `ORDER BY` や `min`/`max` で効果。

## 複数インデックスの利用
- 複数 `MATCH` を含む場合、複数インデックスが使われることがある。

## null と型
- インデックスは `null` を格納しない。
- `IS NOT NULL` や型述語（`IS :: STRING NOT NULL`）で利用が有効化。
- 文字列/ポイントの型制約により Text/Point の利用範囲が拡張。

## 追加の指針
- 高頻度の検索プロパティや高カーディナリティに優先度。
- 書き込み性能への影響を考慮。
- `SHOW INDEXES` の `lastRead`/`readCount`/`trackedSince` で不要インデックスを見極め。
