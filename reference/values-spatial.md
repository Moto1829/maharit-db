# Spatial values

Source: https://neo4j.com/docs/cypher-manual/current/values-and-types/spatial/

## POINT型
- 2D/3D の座標 + CRS（座標参照系）。
- `POINT` / `LIST<POINT>` はプロパティ保存・索引化可能。

## CRS
- Geographic: `wgs-84` (SRID 4326), `wgs-84-3d` (4979)
- Cartesian: `cartesian` (7203), `cartesian-3d` (9157)
- CRS間は比較不可、暗黙変換なし。

## 生成
- `point({longitude, latitude})` または `point({x, y})` でCRSを推定。
- 3Dは `height`/`z` を追加。

## 比較
- `<=` 等の比較演算は不可（`null`）。
- `point.distance()` / `point.withinBBox()` で距離/範囲判定。

## 索引
- Point index は距離/バウンディングボックス最適化。
- Range index は等価検索向け。
