# maharit-core

グラフのデータ構造と、それを扱うアルゴリズム。他のクレートはすべてこれに依存する。

## ファイル構成

- `graph.rs`: 単一スレッド向けの `Graph`（密な Vec）と `Node` / `Edge` / `EdgeEnds`。プロパティは `Arc<HashMap>`（複製を安くするため）
- `concurrent_graph.rs`: サーバーが使う `ConcurrentGraph`（DashMap）。隣接リストはノードごと・種別ごとに分け、相手側のノード ID を持つ。種別名は共有（`Arc<str>`）
- `graph_backend.rs`: `GraphBackend` トレイト。executor・ACL のラッパー・`RecordingGraph` はこれを通してグラフを触る
- `property.rs`: `PropertyValue` と `temporal`（日付・期間の計算。範囲外は `None` を返す）
- `property_index.rs` / `index.rs`: プロパティ索引とラベル索引。`constraint.rs`: 制約。`fulltext.rs`: BM25 の全文索引
- `traversal.rs` / `algorithms.rs`: 経路探索とグラフアルゴリズム（rayon で並列）

## 知らないと間違えること

- ラベルの変更は `add_node_label` / `remove_node_label` を通す。ラベル索引を更新するので、`Node.labels` を直接書き換えると `MATCH (n:Label)` が取りこぼす
- `GraphBackend` にメソッドを足すときは、既定の実装を書くか、実装 4 つ（`Graph`、`ConcurrentGraph`、`maharit-query` の `AclBackend`、`maharit-server` の `RecordingGraph`）すべてに足す。`AclBackend` は見えないノード・関係を隠し、`RecordingGraph` は書き込みを記録するので、ただ委ねるだけでよいかを考える
- `ConcurrentGraph` は読み取りと書き込みが同時に走る。エッジは「本体を入れてから隣接リストに足す」「隣接リストから外してから本体を消す」の順にして、隣接リストから見つけたエッジが必ず引けるようにしている
- ノード・エッジの ID は振り直さない（WAL とレプリケーションが ID で対象を指す）
