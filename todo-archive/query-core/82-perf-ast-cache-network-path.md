# query-core/82: AST キャッシュをネットワーク経路に配線（性能改善 3/4）

## 概要
TCP サーバーの `execute_query` / `execute_query_with_tx` / `execute_streaming_query`
はリクエストごとに `Parser::new(query).parse()` で毎回パースしていた。
`cache.rs` に `AstCache` があるのに hot path で未使用だった。

## 対応
- `TcpServer` に共有 `ast_cache: Arc<Mutex<AstCache>>`（容量 512）を追加し、
  全コンストラクタ・`handle_connection`・3 つの実行関数へ配線。
- 各実行関数の inline パースを `ast_cache.lock().get_or_parse(query)` に置換。
  キーは正規化済みクエリ文字列。パースエラーはキャッシュしない。
- ストリーミング経路は await をまたぐ MutexGuard を避けるため、ロック結果を
  ローカルに束縛してからロックを解放。

## 効果
- 同一クエリ文字列の繰り返し（point-lookup 反復・アプリの定型クエリ）でパースを省略。
- AST（構文木）のみをキャッシュし結果集合はキャッシュしないため、
  書き込み後の同一読み取りクエリは最新のグラフ状態を反映する（テストで確認）。
- `$param` は AST に値を持たず実行時に束縛するため、クエリ文字列単位のキャッシュで安全。

## ステータス
完了（server 238 テストパス、キャッシュ整合テスト追加）
