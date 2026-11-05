# タスク 140: docs 英語翻訳 第 3 弾 — 運用 / 高度なトピック / API / アルゴリズム / 可視化

前提: タスク 139 完了。進め方はタスク 138 と同じ。

## 対象
- [ ] `operations/`: server-config / tls / auth / acl / metrics / tracing / backup / docker
- [ ] `advanced/`: transactions / replication / sharding / query-optimization / parallelization
- [ ] `api/`: rust-client / python-client / protocol
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
