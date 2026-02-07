# Index hints for the Cypher planner

Source: https://neo4j.com/docs/cypher-manual/current/indexes/search-performance-indexes/index-hints/

## 概要
- `USING` 句でプランナの開始点や結合を制御する。
- 3 種類: Index hints / Scan hints / Join hints。
- 強制は結果を変えてはならないため、適用可能な型に制約。
- 高度なチューニング用途。誤用は性能劣化の原因。

## Index hints
- ノード: `USING [RANGE|TEXT|POINT] INDEX v:Label(prop)`
- ノード（SEEK 指定）: `USING [RANGE|TEXT|POINT] INDEX SEEK v:Label(prop)`
- 関係: `USING [RANGE|TEXT|POINT] INDEX v:TYPE(prop)`
- 関係（SEEK 指定）: `USING [RANGE|TEXT|POINT] INDEX SEEK v:TYPE(prop)`
- 型指定なしの場合、利用可能なインデックス型なら可。
- 複数指定は複数の開始点と結合を強制し得る。

## Scan hints
- `USING SCAN v:Label` / `USING SCAN v:TYPE`
- インデックスを使わずにラベル/タイプスキャンを強制。
- 低選択性やフルスキャンが有利なケースで使用。

## Join hints
- `USING JOIN ON v`
- Join の結合ノードを強制。
- `OPTIONAL MATCH` で `NodeLeftOuterHashJoin`/`NodeRightOuterHashJoin` を誘導可能。
- 追加の開始点を強制するため、計画が悪化する可能性がある。

## 注意点
- 型指定ヒントは「結果が変わらない」ことが前提。
- 結果の正しさは保ちつつ計画のみ影響。
