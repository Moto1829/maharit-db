# サーバー: RwLock<Graph> の粒度を細かくする

## 概要

グラフ全体を `Arc<RwLock<Graph>>` 1つで管理しているため、
並行接続が増えると書き込みが全ての読み取りをブロックする。
`DashMap` またはパーティション分割でロック競合を解消する。

## 現状の問題

```rust
pub struct TcpServer {
    graph: Arc<RwLock<Graph>>,  // グラフ全体が1つのロック
}

// クエリ実行中ずっとロックを保持
let response = execute_query(&graph, &query).await;  // 他の全接続をブロック
```

書き込みクエリが実行中は、読み取りのみのクエリも待機させられる。

## 実装内容

### 案A: DashMap の導入（未実装）

- [x] `maharit-core` の `nodes` / `edges` を `DashMap<NodeId, Node>` に変更
  （`ConcurrentGraph` として `concurrent_graph.rs` に実装）
- [x] 読み取り専用クエリは複数同時実行可能にする（DashMap の sharded read locks）
- [x] 書き込みクエリは影響するシャードのみロック（DashMap の per-entry write guards）

### 案B: 読み取り専用エグゼキューター（実装済）

- [x] 読み取りクエリに `graph.read()` を使用（共有ロック）
- [x] `Executor::new_readonly(&Graph)` で不変参照からエグゼキューターを生成
- [x] 書き込みクエリは `graph.write()` で排他ロック
- [x] `is_read_only()` で読み取り/書き込みクエリを分類

## 期待効果

- 読み取り並行性: 接続数に比例したスループット向上（現状は直列化）
- 書き込みレイテンシ改善（シャード化により影響範囲限定）
- 並行スループット +2〜3倍（読み取り多数のワークロード）

## 注意

- インデックス（`LabelIndex`, `PropertyIndex`, `FulltextManager`）も
  同様に並行アクセス対応が必要
- トランザクション整合性への影響を慎重に確認すること

## 対象クレート

`maharit-core`, `maharit-server`
