# Functions

Source: https://neo4j.com/docs/cypher-manual/current/functions/

## 概要
- Cypherの全関数カテゴリを一覧化した章。
- `SHOW FUNCTIONS` で一覧取得可能。
- 文字列入力を取る関数はUnicode文字単位で処理する。

## カテゴリ
- Aggregating functions: `avg`, `collect`, `count`, `min`, `max`, `percentileCont`, `percentileDisc`, `stDev`, `stDevP`, `sum`。
- Database functions: `db.nameFromElementId`。
- Graph functions: `graph.byElementId`, `graph.byName`, `graph.names`, `graph.propertiesByName`。
- List functions: `keys`, `labels`, `nodes`, `relationships`, `range`, `reduce`, `reverse`, `tail`, `toBooleanList`, `toFloatList`, `toIntegerList`, `toStringList` など。
- LOAD CSV functions: `file`, `linenumber`。
- Mathematical functions: logarithmic / numeric / trigonometric（`cosh`, `sinh`, `tanh`, `coth`等はCypher 25）。
- Predicate functions: `all`, `allReduce`（Cypher 25）、`any`, `exists`, `isEmpty`, `none`, `single`。
- Scalar functions: `char_length`, `character_length`, `coalesce`, `elementId`, `id`(非推奨), `length`, `properties`, `randomUUID`, `size`, `timestamp`, `toBoolean/Float/Integer`, `valueType` など。
- String functions: `btrim`, `left`, `lower`, `ltrim`, `normalize`, `replace`, `reverse`, `right`, `rtrim`, `split`, `substring`, `toLower`, `toString`, `toUpper`, `trim`, `upper`。
- Spatial functions: `point`, `point.distance`, `point.withinBBox`。
- Temporal functions: `duration*`, `date*`, `datetime*`, `localdatetime*`, `localtime*`, `time*`, `format`(Cypher 25)。
- User-defined functions: Javaで実装されるスカラー/集約UDF。
- Vector functions: `vector`, `vector.similarity.*`, `vector_dimension_count`, `vector_distance`, `vector_norm`（Cypher 25）。
