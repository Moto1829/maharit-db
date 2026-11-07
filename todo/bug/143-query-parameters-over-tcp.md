# タスク 143: TCP 経由でクエリパラメータを送れない / Python の query_with_params が値をエスケープしない

## 現象
- `maharit-query` は `$param`（`Executor::execute_with_params`）に対応しているが、TCP プロトコルのリクエストにパラメータの欄がなく、サーバーもクライアントも送れない。
- `docs/quickstart.md` は Rust クライアントの `client.query_with_params(...)` を「SQL インジェクション対策」として紹介していたが、このメソッドは存在しない（2026-11-07 に記述を削除し、注意書きに置き換えた）。
- Python クライアント `clients/python/maharit/client.py` の `query_with_params` は、値を `"{value}" AS key` の形でクエリ文字列に**エスケープせずに**埋め込んで `WITH` を前置している。値に `"` を含めるとクエリを書き換えられる（インジェクション）。名前に反して安全なパラメータ化になっていない。

## 作業
- [ ] TCP プロトコルのクエリリクエストに省略可能な `params`（JSON オブジェクト）を追加し、サーバーで `execute_with_params` に渡す（`feat:`、後方互換）
- [ ] Rust クライアントに `query_with_params` / `execute_with_params` を追加
- [ ] Python クライアントの `query_with_params` をプロトコルの `params` を使う実装に置き換える（それまでの間は文字列のエスケープを入れる。`security:`）
- [ ] `docs/api/` のプロトコル仕様と quickstart の注意書きを更新
