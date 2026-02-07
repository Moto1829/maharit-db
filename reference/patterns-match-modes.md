# Match Modes

Source: https://neo4j.com/docs/cypher-manual/current/patterns/match-modes/

## 概要
- マッチモードは**関係の再訪可否**を決める（ノードは常に再訪可能）。

## DIFFERENT RELATIONSHIPS（既定）
- 関係は1回しか出現できない。
- 方向は関係なく「同一関係の再訪」を禁止。
- Cypher 25で明示キーワードが追加。

## REPEATABLE ELEMENTS（Cypher 25）
- ノード/関係の再訪を許可。
- 無限解を避けるため**上限付き量指定が必須**（`*`,`+`,`{n,}`は不可）。

## 注意点
- `REPEATABLE ELEMENTS` は結果数が爆発しやすい。
- `MATCH` にのみ指定可能（`MERGE`不可）。

## 参照
- DIFFERENT RELATIONSHIPS ルール: https://neo4j.com/docs/cypher-manual/current/patterns/reference/#match-modes-rules-different-relationships
- REPEATABLE ELEMENTS ルール: https://neo4j.com/docs/cypher-manual/current/patterns/reference/#match-modes-rules-repeatable-elements
