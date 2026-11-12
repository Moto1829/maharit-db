---
title: クエリ最適化（EXPLAIN / PROFILE）
parent: 高度なトピック
nav_order: 4
---

# クエリ最適化（EXPLAIN / PROFILE）

`EXPLAIN` と `PROFILE` で、クエリの実行計画（クエリプラン）を確認できます。

## EXPLAIN

クエリを実行せずに、実行計画を表示します。各行が 1 つの処理（オペレーター）で、上から順に実行されます。

```cypher
EXPLAIN MATCH (p:Person {name: "Alice"})-[:KNOWS]->(f:Person)
RETURN p.name, f.name
```

出力例:

```text
Operator                    Est. Rows     Cost Details
------------------------------------------------------------------------
NodeByLabelScan                     1        1
Expand                              2        1 :KNOWS
Projection                          2        1
```

- `Est. Rows`: そのオペレーターが出すと見積もった行数
- `Cost`: 見積もりの処理量（相対値）
- `Details`: 対象のラベル・リレーションシップの型・集計のキーなど

## PROFILE

クエリを実際に実行し、実行計画とあわせて結果の行数と実行時間を表示します。

```cypher
PROFILE MATCH (p:Person {name: "Alice"})-[:KNOWS]->(f:Person)
RETURN p.name, f.name
```

出力例:

```text
Operator                    Est. Rows     Cost  Actual Rows    Time (us)
------------------------------------------------------------------------
NodeByLabelScan                     1        1            -            -
Expand                              2        1            -            -
Projection                          2        1            2          133
Rows: 2, Time: 133 us
```

実際の行数と時間は、クエリ全体の値を最後のオペレーターに表示します（オペレーターごとの実測はありません）。

## オペレーター

| オペレーター | 説明 |
|------------|------|
| `NodeByLabelScan` | ラベルを持つノードを走査 |
| `NodeByLabelScan+Filter` | ラベルで走査しながら条件で絞り込む（フィルタのプッシュダウン） |
| `IndexSeek` | プロパティインデックスで等価条件を検索 |
| `IndexRangeScan` | プロパティインデックスで範囲条件を検索 |
| `Expand` / `ExpandReverse` | リレーションシップを辿って隣のノードへ（順方向 / 逆方向） |
| `Filter` | 条件で絞り込む |
| `HashAggregation` / `EagerAggregation` | 集計 |
| `Sort` | 並べ替え |
| `Limit` | 行数を制限 |
| `Projection` | 返す列を作る |
| `Unwind` / `Union` / `CallSubquery` / `ProcedureCall` | `UNWIND` / `UNION` / `CALL { … }` / `CALL proc()` |
| `CreateNode` / `SetProperty` / `RemoveProperty` / `Delete` / `Merge` / `Foreach` | 書き込みの各処理 |

## クエリのキャッシュ

サーバーは、パースしたクエリ（構文木）を、クエリ文字列ごとに最大 512 件キャッシュします（古いものから捨てる LRU）。同じ文字列のクエリを繰り返すと、パースを省けます。値はパラメータ（`$name`）で渡すと、値が変わっても同じ文字列になり、キャッシュが効きます。キャッシュの状態を見たり消したりする手段はありません。

## 最適化の手法

### 1. インデックスの活用

最も効果的な最適化はインデックスの適切な使用です。

```cypher
// インデックスなし（NodeByLabelScan + Filter）
MATCH (n:Person) WHERE n.email = "alice@example.com" RETURN n

// インデックスを作成
CREATE INDEX FOR (n:Person) ON (n.email)

// インデックスあり（IndexSeek）→ 速い
MATCH (n:Person) WHERE n.email = "alice@example.com" RETURN n
```

### 2. フィルタのプッシュダウン

フィルタ条件はできるだけ早い段階（パターンのマッチング直後）に適用されます。

```cypher
// 推奨: 条件を MATCH のパターンに含める
MATCH (p:Person {age: 30})-[:KNOWS]->(f:Person)
RETURN f.name

// 同様に動作（フィルタプッシュダウンが有効）
MATCH (p:Person)-[:KNOWS]->(f:Person)
WHERE p.age = 30
RETURN f.name
```

### 3. 可変長パスは絞り込んだ側から辿る

パターンは左端のノードから辿ります。可変長パスでは、条件で少数に絞り込めるノードを左端に置くと、辿る経路が少なくなります。

```cypher
// 推奨: 1 件に絞り込める Alice から辿る
MATCH (p:Person {name: "Alice"})-[:KNOWS*1..5]->(f:Person)
RETURN f.name

// 非推奨: すべての Person から辿ってから、終点で Alice に絞り込む
MATCH (f:Person)-[:KNOWS*1..5]->(p:Person {name: "Alice"})
RETURN f.name
```

### 4. WITH による中間結果の制限

`LIMIT` や集計を `WITH` に組み合わせて、早い段階で結果を絞り込みます。

```cypher
// 上位 100 人に絞り込んでからさらに処理
MATCH (p:Person)
WHERE p.age > 20
WITH p
ORDER BY p.score DESC
LIMIT 100
MATCH (p)-[:KNOWS]->(f:Person)
RETURN p.name, collect(f.name) AS friends
```

### 5. 可変長パスへの注意

可変長パス（`*n..m`）は組み合わせ爆発を引き起こす可能性があります。

```cypher
// 注意: ホップ数が大きいと遅くなる
MATCH (a:Person)-[:KNOWS*1..10]->(b:Person)
RETURN b.name

// 推奨: ホップ数を制限する
MATCH (a:Person)-[:KNOWS*1..3]->(b:Person)
RETURN b.name
```
