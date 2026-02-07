# Spatial functions

Source: https://neo4j.com/docs/cypher-manual/current/functions/spatial/

## 関数
- `point(map)`：CRS/座標から `POINT` を生成。
- `point.distance(p1, p2)`：同一CRS間の距離。
- `point.withinBBox(p, lowerLeft, upperRight)`：BBox判定。

## 重要ルール
- CRSが異なると `null`。
- 2D/3Dの距離計算はCRSに依存。
