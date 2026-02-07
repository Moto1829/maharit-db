# Patterns Syntax and Semantics

Source: https://neo4j.com/docs/cypher-manual/current/patterns/reference/

## Node patterns
- 構文: `(` [nodeVariable] [labelExpression] [propertyKeyValueExpression] [WHERE] `)`
- 述語はラベル式/プロパティKV/`WHERE` の3種。
- 変数は未束縛なら新規束縛、束縛済みならフィルタとして作用。

## Relationship patterns
- 構文: `-[]-` / `-[]->` / `<-[]-`。
- 述語はラベル式/プロパティKV/`WHERE`、方向指定。

## Label expressions
- `&`（AND）, `|`（OR）, `!`（NOT）, `%`（ワイルドカード）。
- 優先順位: `()` > `!` > `&` > `|`。
- 動的ラベル/型: `:$()` で式/パラメータから指定。

## Property key-value expressions
- `{ key: value }` は `WHERE` の等価条件に相当。

## Path patterns
- 必ずノードで開始/終了。
- ノード/関係が交互。
- 量指定パターン `{m,n}` で可変長。

## Quantified path patterns
- 量指定対象は**関係を含むパス**（単一ノード不可）。
- 量指定内変数は**グループ変数**として外側でリスト化。

## Quantified relationships
- 量指定パスの簡略記法。両端にノード必須。

## Quantifiers
- `*`= `{0,}`、`+`=`{1,}`、`{m,n}`。
- 量指定 `{1}` でも**グループ変数化**され固定長と同一ではない。

## Variable-length relationships（非GQL）
- `-[*m..n]->` の旧構文。
- 量指定位置/意味が量指定パターンと異なる。

## Shortest paths
- セレクタ: `SHORTEST k`, `ALL SHORTEST`, `SHORTEST k GROUPS`, `ANY k`, `ALL`。
- 選択順序: **パターン一致 → 事前フィルタ → セレクタ → 事後フィルタ**。
- セレクタ使用時は**1パスパターンのみ**（`DIFFERENT RELATIONSHIPS`）。

## shortestPath()/allShortestPaths()
- 旧関数。単一可変長関係のみ許容。
- 非GQL準拠。

## Graph patterns
- `pathPattern` をカンマ結合。
- 共有変数がない場合は直積。

## Match modes
- `DIFFERENT RELATIONSHIPS` と `REPEATABLE ELEMENTS`。
- `REPEATABLE ELEMENTS` は量指定に上限必須。

## Node pattern pairs
- 量指定パターンの展開により、隣接ノードパターンが統合される。
- 直接 `(a)(b)` の書式は不可だが、展開で生じる場合は有効。
