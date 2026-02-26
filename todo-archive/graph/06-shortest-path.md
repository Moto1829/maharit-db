# 最短経路アルゴリズム

## 概要
重み付きグラフにおける最短経路を求めるアルゴリズムを実装する。

## 実装内容

### Dijkstra法
- [x] 基本実装（BinaryHeapを使用）
- [x] 単一始点から全ノードへの最短距離
- [x] 経路の復元

### A*アルゴリズム
- [x] ヒューリスティック関数のインターフェース
- [x] 基本実装

### 重みの取得
- [x] エッジプロパティからの重み取得
- [x] デフォルト重み（1.0）
- [x] カスタム重み関数

### 結果の表現
```rust
pub struct ShortestPath {
    pub nodes: Vec<NodeId>,
    pub edges: Vec<EdgeId>,
    pub total_weight: f64,
}
```

## 依存
- `05-graph-traversal.md` の基本探索が完了していること

## 対象クレート
`maharit-core`
