# サーバー: 読み取りクエリの並列実行

## 概要

現状は読み取りクエリ（MATCH）も書き込みクエリ（CREATE/SET/DELETE）も
同じ `graph.write()` で直列化されており、複数クライアントが同時接続しても
クエリが逐次実行される。読み取り専用クエリを `graph.read()` で実行することで
複数クライアントが同時にクエリを実行できるようにする。

## 現状の問題

```rust
// tcp_server.rs
async fn execute_query(graph: &Arc<RwLock<Graph>>, query: &str) {
    let mut g = graph.write().await;   // 全クエリが排他ロック
    let mut executor = Executor::new(&mut g);
    executor.execute(stmt)
}
```

読み取りのみのクエリが書き込みクエリと同じロックを取得するため、
並行接続数が増えても実効スループットが上がらない。

## 実装内容

- [ ] クエリを実行前に「読み取り専用か否か」を判定する関数を実装
  ```rust
  fn is_read_only(stmt: &Statement) -> bool {
      matches!(stmt, Statement::Match(_) | Statement::Explain(_) | Statement::Profile(_))
  }
  ```
- [ ] 読み取り専用クエリは `graph.read().await` で実行
  ```rust
  if is_read_only(&stmt) {
      let g = graph.read().await;   // 複数スレッド同時実行可能
      let executor = ReadOnlyExecutor::new(&g);
      executor.execute(stmt)
  } else {
      let mut g = graph.write().await;
      let mut executor = Executor::new(&mut g);
      executor.execute(stmt)
  }
  ```
- [ ] `ReadOnlyExecutor`（または `Executor::new_readonly(&Graph)`）を追加
  （`&mut Graph` ではなく `&Graph` を受け取るバリアント）
- [ ] `CALL db.*` 系の組み込みプロシージャも読み取り/書き込みで分類

## 期待効果

- 読み取り多数のワークロード（OLAP 系）: **3〜5倍**のスループット向上
- 読み書き混合ワークロード: **1.5〜2倍**
- 書き込み多数のワークロード: 変化なし

## 注意

- `MATCH ... CREATE` 等の複合クエリは書き込みとして分類する
- インデックス・制約マネージャーも `&` / `&mut` を統一する必要がある
- タスク 58（DashMap）と組み合わせると書き込みのボトルネックもさらに解消される

## 依存

- タスク 58（RwLock 粒度改善）と連携すると効果が最大化される

## 対象クレート

`maharit-server`, `maharit-query`
