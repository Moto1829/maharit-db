# タスク 143: TCP 経由でクエリパラメータを送れない / Python の query_with_params が値をエスケープしない

## 現象
- `maharit-query` は `$param`（`Executor::execute_with_params`）に対応しているが、TCP プロトコルのリクエストにパラメータの欄がなく、サーバーもクライアントも送れない。
- `docs/quickstart.md` は Rust クライアントの `client.query_with_params(...)` を「SQL インジェクション対策」として紹介していたが、このメソッドは存在しない（2026-11-07 に記述を削除し、注意書きに置き換えた）。
- Python クライアント `clients/python/maharit/client.py` の `query_with_params` は、値を `"{value}" AS key` の形でクエリ文字列に**エスケープせずに**埋め込んで `WITH` を前置している。値に `"` を含めるとクエリを書き換えられる（インジェクション）。名前に反して安全なパラメータ化になっていない。

## 作業
- [x] TCP プロトコルのクエリリクエストに省略可能な `params`（JSON オブジェクト）を追加し、サーバーで `execute_with_params` に渡す（`feat:`、後方互換）
- [x] Rust クライアントに `query_with_params` / `execute_with_params` を追加
- [x] Python クライアントの `query_with_params` をプロトコルの `params` を使う実装に置き換える（それまでの間は文字列のエスケープを入れる。`security:`）
- [x] `docs/api/` のプロトコル仕様と quickstart の注意書きを更新

## 結果（2026-11-08）
- サーバー: `query` / `streamQuery` に省略可能な `params`（JSON オブジェクト）を追加。`Value::from_json` で変換し `execute_with_params` に渡す。トランザクション内（`txId`）も対応。未定義のパラメータはエラー
- Rust クライアント: `Params` 型、`query_with_params` / `query_with_params_in_tx` / `execute_with_params`（`Client`・プール・`SyncClient`）。空の params は送らないので旧サーバーとも互換
- Python クライアント: `execute` / `query` / `stream` / `Transaction.execute` に `params` 引数。`query_with_params` はクエリ文字列への埋め込みをやめ、`params` フィールドで送る（async も同様）
- docs: `api/protocol.md` を実装どおりに全面書き直し（以前はハンドシェイクや 1 バイトの型などがあり、実在しないプロトコルの記述だった）。quickstart の注意書きを例に戻し、python-client / parameters を更新
- 実機確認: `x"}) DETACH DELETE n //` を値に渡してもデータとして保存・検索されることを、Rust の TCP テストと Python クライアント + 実サーバーで確認
- 別件で見つかったもの: query-clauses/145（`(n $props)` / `$p.key` 未対応）、graph/146（リストのプロパティ未対応）
