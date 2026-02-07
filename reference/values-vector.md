# Vectors

Source: https://neo4j.com/docs/cypher-manual/current/values-and-types/vector/

## 概要
- `VECTOR` は固定長の数値配列（`INTEGER`/`FLOAT`）を単一値として扱う。
- **Enterprise/Aura**でのプロパティ保存がサポート（Neo4j 2025.10）。

## 仕様
- 次元は 1〜4096。
- 座標型: `INTEGER8/16/32/64`, `FLOAT32/64`。
- リスト内にベクタは保存不可。

## 生成
- `vector(list, dimension, type)` 関数。

## ドライバ対応
- ドライバ v6.0未満では `VECTOR` がプレースホルダーMAPで返却される。

## 型強制
- 座標型により変換/丸め/オーバーフロー判定。
- `INTEGER`→`FLOAT` や `FLOAT`→`INTEGER` は情報損失があり得る。

## スーパータイプ
- `VECTOR`, `VECTOR<TYPE>`, `VECTOR(DIM)` が上位型。

## 参照
- ベクタ関数: https://neo4j.com/docs/cypher-manual/current/functions/vector/
- ベクタインデックス: https://neo4j.com/docs/cypher-manual/current/indexes/semantic-indexes/vector-indexes/
