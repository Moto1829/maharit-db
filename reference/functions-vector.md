# Vector functions

Source: https://neo4j.com/docs/cypher-manual/current/functions/vector/

## vector()
- `vector(list, dimension, type)` でVECTOR生成。
- `dimension` は1〜4096。
- `null/NaN/Infinity` は不可。

## 類似度/距離
- `vector.similarity.cosine` / `vector.similarity.euclidean`
- `vector_distance(vector1, vector2, metric)`（EUCLIDEAN, MANHATTAN, COSINE, DOT, HAMMING 等）
- `vector_norm(vector, metric)`（2025.20）

## サイズ
- `vector_dimension_count(vector)`（2025.10）。

## 備考
- 演算は float32。
- 入力ベクタは次元一致が必須。
