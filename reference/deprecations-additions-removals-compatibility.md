# Additions, deprecations, removals, and compatibility

Source: https://neo4j.com/docs/cypher-manual/current/deprecations-additions-removals-compatibility/

## 概要
- Cypherは継続的に拡張/変更され、Neo4jリリースごとに追加・更新・非推奨・削除が明記される。
- Cypher 25はNeo4j 2025.06で導入。Cypher 5は凍結。
- Cypher 25で削除された機能は、`CYPHER 5`指定やDB既定言語がCypher 5であれば引き続き利用可能。

## Neo4j 2026.01
- **Updated in Cypher 25**
	- VECTORインデックスが**複数ラベル/関係タイプ**や**追加プロパティによるフィルタ**に対応。
- **New in Cypher 25**
	- クエリキャッシュ対象の**クエリサイズ上限**を導入。

## Neo4j 2025.11
- **Updated in Cypher 25**
	- 新しいコレクション系関数: `coll.distinct`, `coll.flatten`, `coll.indexOf`, `coll.insert`, `coll.max`, `coll.min`, `coll.remove`, `coll.sort`。
	- `MERGE`性能向上の新オペレーター `MergeInto`, `MergeUniqueNode`。
	- 動的ラベル/型のインデックス活用改善（ただし制約あり: rangeの正確一致のみなど）。

## Neo4j 2025.10
- **New in Cypher 25**
	- `VECTOR`値型と`vector()`関数、`vector_dimension_count()`、`vector_distance()`、`vector_norm()`。
	- VECTORプロパティ型制約。
- **Updated in Cypher 25**
	- `toFloatList()`/`toIntegerList()`と`vector.similarity.*`がVECTOR引数を受け入れ。
	- `date()`, `datetime()`, `localdatetime()`, `localtime()`, `time()`, `duration()`で**パターン指定パース**が可能。

## Neo4j 2025.09
- **New in Cypher 25**
	- `format()`関数で日時/期間の書式化。
	- `LockNodes`オペレーター追加。

## Neo4j 2025.08
- **Updated in Cypher 25**
	- `NEXT`がUNION/CALL内集約に対応。
	- 動的ラベル/型のトークンルックアップインデックス活用の改善。
- **New in Cypher 25**
	- `allReduce()`関数追加（パス探索の早期枝刈りに最適化）。

## Neo4j 2025.07
- **Updated in Cypher 25**
	- ラベル式が許される箇所で動的ラベル/型の参照が可能に。

## Neo4j 2025.06
- **Removed in Cypher 25**
	- `SET n = r` のようにNODE/RELATIONSHIPをMAPとして扱う構文を削除。`properties()`を使用。
	- `MERGE`内で同一パターン内の別エンティティプロパティを参照する構文を削除。
	- `CREATE ... INDEX/CONSTRAINT ... OPTIONS { indexProvider: ... }` の指定を削除。
	- いくつかのプロシージャ（例: `db.index.vector.createNodeIndex` 等）削除。
	- グラフ参照の部分的バッククォート構文（`USE composite.`形式）削除。
	- 一部の非推奨Unicode/記号が**未引用識別子**として禁止。
- **Deprecated in Cypher 25**
	- `CREATE DATABASE ... OPTIONS { existingData: ... }` の `existingData`。
- **Updated in Cypher 25**
	- `COLLECT`/`COUNT`/`EXISTS`サブクエリ式の外部変数の扱い修正。
	- パラメータ名に拡張識別子文字が使用可能（GQL準拠）。
	- `SHORTEST`/`ANY`パターンでパラメータ利用可能。
	- `SHOW CONSTRAINTS` のフィルタで `PROPERTY` キーワードが追加。
	- `SHOW TRANSACTIONS`の列型/NULL挙動の調整。
	- `graph.byName`/`graph.propertiesByName` の引数でドット含み名の引用が不要に。
	- 読み取り→書き込みの遷移で`WITH`必須が撤廃。
	- `replace()`に上限回数引数追加。
- **New in Cypher 25**
	- DB/エイリアス作成・変更時の `DEFAULT LANGUAGE CYPHER 25` 指定。
	- `SHOW DATABASES` / `SHOW ALIAS` の `defaultLanguage` 返却。
	- `CYPHER 25`クエリオプション。
	- `NEXT`（直列クエリ合成）、`LET`、`FILTER`。
	- `WHEN/ELSE`による条件分岐クエリ。
	- `UNION`と`UNION ALL`を同一クエリで組み合わせるための中括弧。
	- `RETURN ALL`/`WITH ALL`による明示的な重複保持。
	- `MATCH REPEATABLE ELEMENTS` と `MATCH DIFFERENT RELATIONSHIPS`。
