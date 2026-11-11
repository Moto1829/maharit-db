# タスク 140: docs 英語翻訳 第 3 弾 — 運用 / 高度なトピック / API / アルゴリズム / 可視化

前提: タスク 139 完了。進め方はタスク 138 と同じ。

## 対象
- [x] `operations/`: server-config / tls / auth / acl / metrics / tracing / backup / docker
- [x] `advanced/`: transactions / replication / sharding / query-optimization / parallelization
- [x] `api/`: rust-client / python-client / protocol
- [ ] `algorithms/`: shortest-path / all-paths / centrality / connected-components / cycle-detection
- [ ] `visualization/`: export / ascii / web-app

## 注意
- `api/protocol.md` は他言語でクライアントを書く人が読む仕様なので、JSON のフィールド名・型・例は原文と完全に一致させる
- `api/python-client.md` は `clients/python/README.md`（既に英語）と内容が重なる。英語版は README へのリンクで済ませてよいか判断する
- CLI フラグやデフォルト値は、翻訳の前に `maharit server --help` の出力と照合し、日本語側が古ければ先に日本語を直す

## 完了条件
- docs の全ページに `.en.md` があり、`fallback_to_default` で日本語が表示されるページがなくなる
- `mkdocs build --strict` と `docs_cypher_examples` テストが通る
- 全ページ揃った時点で、英語をサイトのデフォルト（ルート）にするかを判断する（変える場合は日本語を `/ja/` に移し、旧 URL からのリダイレクトを用意する）

## 進捗（2026-11-12）
- operations / advanced / api を英訳（operations は task148 で照合済み）。訳す前に日本語版の誤りを直した:
  sharding（Rust API の関数名・引数が実在しない → 実際の API に書き直し、コンパイル確認）、
  query-optimization（EXPLAIN / PROFILE の出力例が架空、db.cache.* プロシージャは無い）、
  rust-client（pool::Pool・ClientBuilder・ClientError::QueryError・row.get_string は無い → 実際の API に、コンパイル確認）、
  python-client（PyPI 未公開、async with AsyncClient.connect が動かなかった → クライアント側を修正 435b1f2c）、parallelization（コマンド名）
- 残り: algorithms（5 ページ。CALL db.pageRank などのプロシージャも graph.add_node などの Rust API も実在しないため全面的に書き直しが必要）、visualization（3 ページ）
