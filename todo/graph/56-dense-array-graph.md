# グラフ構造: HashMap を稠密配列に変更

**Status**: Completed

## 概要

`NodeId` / `EdgeId` は連番整数なのに `HashMap` で管理しているため、
ハッシュ計算・キャッシュミスが発生している。`Vec<Option<T>>` に変えることで
メモリ使用量とアクセス速度を改善する。

## 現状の問題

```rust
pub struct Graph {
    nodes: HashMap<NodeId, Node>,   // ハッシュ計算 + キャッシュミス
    edges: HashMap<EdgeId, Edge>,
    outgoing_edges: HashMap<NodeId, Vec<EdgeId>>,
    incoming_edges: HashMap<NodeId, Vec<EdgeId>>,
}
```

NodeId / EdgeId は単調増加する整数であるため HashMap は不要。

## 実装内容

- [x] `nodes: HashMap<NodeId, Node>` → `nodes: Vec<Option<Node>>` に変更
- [x] `edges: HashMap<EdgeId, Edge>` → `edges: Vec<Option<Edge>>` に変更
- [x] `outgoing_edges` / `incoming_edges` も `Vec<Vec<EdgeId>>` に変更
- [x] 削除済みスロットを再利用するフリーリスト（`Vec<NodeId>`）を追加
- [x] `Graph::nodes()` / `edges()` イテレータで `None` スロットをスキップ
- [x] 既存の全テスト・全クレートが通ることを確認

## 期待効果

- メモリ使用量 -30%
- ノード/エッジアクセス速度 +30%（ハッシュ計算・衝突解決の排除）
- CPU キャッシュヒット率向上

## 対象クレート

`maharit-core`
