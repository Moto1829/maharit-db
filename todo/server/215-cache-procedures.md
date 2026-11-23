# タスク 215: クエリキャッシュのプロシージャ（db.cache.info / db.cache.stats / db.cache.clear）

## 背景（2026-11-23、task127 の残り）
task127（task148）で docs から外したプロシージャ。サーバーは `AstCache` を持つ（`maharit-server/src/tcp_server/state.rs`）が、
中身や当たり率を外から見る手段も、消す手段も無い。

※ 実装するかどうかは後で決める。決めるまでは着手しない。

## 候補
- `CALL db.cache.info() YIELD name, capacity, size` — キャッシュごとの容量と件数
- `CALL db.cache.stats() YIELD name, hits, misses, evictions`
- `CALL db.cache.clear()` — admin のみ

## 課題（task216 / task217 と共通）
- プロシージャは `maharit-query` の Executor で解決しているが、キャッシュ・バックアップ・レプリケーションはサーバー側の状態で、
  Executor からは見えない。サーバーからプロシージャを差し込む仕組み（名前 → ハンドラの登録）を最初に作る。
  このタスクを最初にやるなら、その仕組みもここで作る
- 権限: 見るだけなら reader、`clear` は admin。`is_read_only` と RBAC の判定にどう載せるか
- シャーディング時（全シャードの行を連結）と、HTTP の `/metrics` との重複

## 作業
- [ ] サーバー側プロシージャの登録の仕組み（未作成なら）
- [ ] `AstCache` などに当たり・外れの数を数える計測を足す
- [ ] 3 つのプロシージャを追加し、権限を確かめるテストを書く
- [ ] docs（運用の章、日英）に追加
