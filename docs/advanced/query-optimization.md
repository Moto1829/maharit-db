---
title: クエリ最適化（EXPLAIN / PROFILE）
parent: 高度なトピック
nav_order: 4
---

# クエリ最適化（EXPLAIN / PROFILE）

MaharitDB はクエリの実行計画を確認するための `EXPLAIN` および `PROFILE` をサポートしています。

## EXPLAIN

クエリを実際に実行せずに、実行計画（クエリプラン）を表示します。

```cypher
EXPLAIN MATCH (p:Person {name: "Alice"})-[:KNOWS]->(f:Person)
RETURN p.name, f.name
```

出力例：

```
Query Plan:
  NodeIndexSeek[p:Person(name="Alice")]
    └── Expand[p-[:KNOWS]->f]
          └── Filter[f:Person]
                └── Return[p.name, f.name]

Estimated rows: 5
```

## PROFILE

クエリを実際に実行し、各ステップの実測値を表示します。

```cypher
PROFILE MATCH (p:Person {name: "Alice"})-[:KNOWS]->(f:Person)
RETURN p.name, f.name
```

出力例：

```
Query Plan (Actual):
  NodeIndexSeek[p:Person(name="Alice")]       rows=1  time=0.2ms
    └── Expand[p-[:KNOWS]->f]                rows=3  time=0.5ms
          └── Filter[f:Person]               rows=3  time=0.1ms
                └── Return[p.name, f.name]   rows=3  time=0.1ms

Total time: 0.9ms
Rows returned: 3
```

## クエリプランのノード

| プランノード | 説明 |
|------------|------|
| `NodeScan` | すべてのノードをスキャン（最も遅い） |
| `NodeByLabelScan` | ラベルでノードをスキャン |
| `NodeByLabelScan+Filter` | ラベルスキャン + フィルタのプッシュダウン |
| `IndexSeek` | プロパティインデックスで等値検索（最も速い） |
| `IndexRangeScan` | プロパティインデックスで範囲検索 |
| `ExpandAll` | エッジをたどって隣接ノードを展開 |
| `Filter` | フィルタ条件を適用 |
| `EagerAggregation` | 集計を実行 |
| `Sort` | 結果をソート |
| `Limit` | 結果数を制限 |
| `ProduceResults` | 結果を返す |

## クエリキャッシュ

MaharitDB はパースされたクエリプランをキャッシュします。同じクエリが繰り返し実行される場合、パース処理をスキップして高速化されます。

```cypher
// キャッシュ統計の確認
CALL db.cache.stats()
YIELD hit_count, miss_count, eviction_count, cache_size
RETURN hit_count, miss_count, hit_count * 1.0 / (hit_count + miss_count) AS hit_ratio
```

キャッシュはグラフの統計（ノード数・エッジ数）が大きく変化した場合に無効化されます。

## 最適化の手法

### 1. インデックスの活用

最も効果的な最適化はインデックスの適切な使用です。

```cypher
// インデックスなし（AllNodesScan）
MATCH (n:Person) WHERE n.email = "alice@example.com" RETURN n

// インデックスを作成
CREATE INDEX FOR (n:Person) ON (n.email)

// インデックスあり（NodeIndexSeek）→ 速い
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

## クエリプランキャッシュの管理

```cypher
// クエリプランキャッシュをクリア
CALL db.cache.clear()

// キャッシュサイズを確認
CALL db.cache.info()
YIELD max_size, current_size, hit_count, miss_count
RETURN max_size, current_size, hit_count, miss_count
```
