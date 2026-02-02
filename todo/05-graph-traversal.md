# グラフ探索アルゴリズム

## 概要
グラフを探索するための基本アルゴリズムを実装する。

## 実装内容

### 基本探索
- [x] 幅優先探索（BFS）
- [x] 深さ優先探索（DFS）
- [x] イテレータベースのAPI

### パス探索
- [x] 2ノード間のパス存在確認
- [ ] 全パスの列挙
- [x] 最短パス（ホップ数）
- [ ] `shortestPath((a)-[*]->(b))` 関数
- [ ] `allShortestPaths((a)-[*]->(b))` 関数

### フィルタリング
- [x] ラベルによるフィルタ
- [x] エッジタイプによるフィルタ
- [x] 深さ制限

### API設計
```rust
// 例
graph.traverse(start_node)
    .bfs()
    .with_edge_filter(|e| e.label == "KNOWS")
    .max_depth(3)
    .collect::<Vec<_>>();
```

## 対象クレート
`maharit-core`
