# Graph functions

Source: https://neo4j.com/docs/cypher-manual/current/functions/graph/

## graph.names()
- 現在のコンポジットDBのグラフ名一覧を返す。

## graph.propertiesByName(name)
- グラフ（エイリアス）に付与されたプロパティを返す。

## graph.byName(name)
- `USE` 句でグラフ参照を解決（コンポジットDBのみ）。

## graph.byElementId(elementId)
- 要素IDからグラフ参照を解決（`USE` 句）。
- 標準DB/コンポジットDB両方で利用可能（制限あり）。
