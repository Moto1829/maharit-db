# Equality, ordering, and comparison of value types

Source: https://neo4j.com/docs/cypher-manual/current/values-and-types/ordering-equality-comparison/

## 等価性
- 同一型同士のみ比較可能。
- `PATH` はノード/関係の列としてリストと等価に比較。
- `null` との `=`/`<>` は常に `null`。

## 順序
- 型ごとに比較階層が定義される。
- `null` は最後に並ぶ。
- `LIST` は辞書順、`MAP` はサイズ→キー順→値順。

## 空間/時間
- `POINT` 同士はCRS/座標で順序付け。
- `TEMPORAL` は型→時刻→タイムゾーンで順序付け。
- `DURATION` は比較演算子での大小比較は `null`。

## ベクタ
- 座標型→次元→要素順で比較。
- `VECTOR` は `LIST` とは別の順序規則。

## 注意
- `POINT` と `VECTOR` は不等号比較の対象外。
