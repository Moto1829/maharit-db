# バグ 174: `SET n.p = null` でプロパティが消えず null が残る

## 現状（2026-11-14、graph/146 の作業中に確認）
```text
CREATE (n:Q {a: 1})
MATCH (n:Q) SET n.a = null RETURN keys(n), n.a   // => ["a"] | null
```
Cypher（Neo4j）では null を設定するとプロパティが削除され、`keys(n)` に残らない。
`CREATE (n {a: null})` も同様にプロパティを作らないのが Cypher の意味。

## 作業
- [x] SET / CREATE / MERGE（ON CREATE / ON MATCH）・`SET n += {a: null}` で null は削除として扱う
- [x] WAL・レプリケーション・undo（削除として記録されること）、プロパティインデックス・全文インデックスの更新
- [x] docs（basics の「プロパティの値」）に書く

## 結果（2026-11-14）
- SET（`n.p = null`、`+=`、`=`）で null の値はプロパティの削除にした（NOT NULL 制約は REMOVE と同じく検査）。CREATE / MERGE の作成時は null のプロパティを作らない（ノード・リレーションシップとも）
- 削除は `remove_node_property` / `remove_edge_property` を通るので WAL・レプリケーション・undo・インデックスは既存の REMOVE と同じ経路
- docs/cypher/basics に追記
