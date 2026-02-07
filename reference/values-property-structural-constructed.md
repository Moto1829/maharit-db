# Property, structural, and constructed values

Source: https://neo4j.com/docs/cypher-manual/current/values-and-types/property-structural-constructed/

## 区分
- **Property types**: プロパティに保存可能（`BOOLEAN`, `INTEGER`, `FLOAT`, `STRING`, `DATE`, `LOCAL/ZONED TIME`, `LOCAL/ZONED DATETIME`, `DURATION`, `POINT`, `LIST`, `VECTOR`）。
- **Structural types**: `NODE`, `RELATIONSHIP`, `PATH`（クエリ結果でのみ扱える）。
- **Constructed types**: `LIST`, `MAP`。

## Property typesの注意
- プロパティに保存できるのは**同種の単純型リスト**のみ（`VECTOR`を含むリストは不可）。
- リストのプロパティには `null` を含められない。
- `VECTOR` のプロパティ保存は **Enterprise/Aura + block format** が必要。
- バイト配列はパススルーサポート（リテラルなし）。

## Structural types
- `NODE`: id, labels, properties。
- `RELATIONSHIP`: id, type, properties, start/end id。
- `PATH`: ノードと関係の交互列。

## Constructed types
- `LIST`: 同種/異種の順序付きコレクション。
- `MAP`: Key（リテラル）と Value（任意型）の集合。

## 型の同義語とNOT NULL
- 型には同義語があり、`NOT NULL`/`!` で非NULL指定。
- 閉じたユニオン型は `NOT NULL` を直接付与不可（内側型の整合が必要）。

## 型正規化
- 型は正規化され、包含関係のある型は吸収される。
- `VECTOR` は座標型/次元の有無によりスーパータイプが成立。

## 参考
- 型判定: https://neo4j.com/docs/cypher-manual/current/expressions/predicates/type-predicate-expressions/
