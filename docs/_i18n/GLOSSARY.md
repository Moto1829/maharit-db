# 英訳の用語集とスタイルガイド

docs の英語版（`foo.en.md`）を書くときの決まりごと。サイトには出さない（`mkdocs.yml` の `exclude_docs`）。
新しい用語を訳したら、この表に追加してからページに使う。

## 進め方

1. 日本語 `foo.md` が正本。英語は同じフォルダに `foo.en.md` として置く。
2. ファイル先頭に、どの時点の日本語を訳したかを書く（鮮度チェックに使う。タスク 141）。
   ```markdown
   ---
   source_commit: 6da4c93d
   ---
   ```
   値は `git log -1 --format=%h -- docs/foo.md`（翻訳に反映した日本語ファイルの最新コミット）。
3. ```cypher ブロックは `//` コメントだけ訳す。クエリ本体は 1 文字も変えない
   （`cargo test -p maharit-query --test docs_cypher_examples` が日本語版との一致を検査する）。
4. ```bash / ```rust / ```text などのコード、コマンド、出力例、エラーメッセージ、JSON は原文のまま。
   ただしコード中の日本語コメントは訳してよい。
5. ページ内リンクは日本語版と同じ相対パス（`basics.md`）で書く。英語版があれば自動的にそちらへ向く。
6. 日本語版に誤りを見つけたら、先に日本語版を直してから訳す。

## スタイル

- 米国英語。です・ます調の丁寧さは、平叙の簡潔な英語にする（"You can …" より命令形や平叙文）。
- 見出しは Sentence case（`## Creating nodes`、`## Full-text indexes`）。固有名・キーワードは原文の大文字小文字。
- Cypher のキーワードは大文字のまま（`MATCH`、`WHERE`）、コード扱い（バッククォート）にする。
- admonition の種類（`!!! note`、`!!! warning`）は変えず、タイトルだけ訳す。
- 「〜できます」「〜します」の羅列は、表や箇条書きを保ったまま訳す。構成（見出し・表・箇条書きの順序）は日本語版と揃える。
- 数値・単位はそのまま（`2.62MB`、`7687`）。日付は ISO 形式（`2026-11-06`）。

## 訳さないもの

MaharitDB、Cypher、BM25、IPADIC、lindera、rustls、Prometheus、OpenTelemetry、tokio、rayon、
クレート名（`maharit-core` など）、バイナリ名（`maharit`、`maharit-viz`）、CLI フラグ、設定キー、関数名・プロシージャ名。

## 用語

### グラフ

| 日本語 | 英語 | 備考 |
|---|---|---|
| グラフ | graph | |
| ノード | node | |
| リレーションシップ | relationship | Cypher 文脈（`[:KNOWS]`、`created_relationships` など） |
| エッジ | edge | 内部実装・Rust API・アルゴリズム文脈（`Edge`、`EdgeId`、`created_edges`）。原文の使い分けに従う |
| ラベル | label | |
| リレーションシップタイプ / エッジタイプ | relationship type / edge type | |
| プロパティ | property | |
| パス | path | |
| 隣接リスト | adjacency list | |
| 有向 / 無向 | directed / undirected | |
| 次数 | degree | 入次数 in-degree / 出次数 out-degree |
| 連結成分 / 強連結成分 | connected component / strongly connected component | |
| 最短経路 | shortest path | |
| 全経路探索 | all-paths search | |
| 中心性 / 媒介中心性 / 近接中心性 | centrality / betweenness centrality / closeness centrality | |
| サイクル検出 | cycle detection | |
| トポロジカルソート | topological sort | |

### クエリ

| 日本語 | 英語 | 備考 |
|---|---|---|
| 句（MATCH 句など） | clause | |
| 文 / ステートメント | statement | |
| 式 | expression | |
| 述語 | predicate | |
| 集計 / 集計関数 | aggregation / aggregate function | |
| 変数の束縛 / バインディング | binding | 「変数を束縛する」= bind a variable |
| パラメータ | parameter | `$param` |
| サブクエリ | subquery | |
| パターン / パターンマッチ | pattern / pattern matching | |
| 可変長パターン | variable-length pattern | |
| リスト内包表記 | list comprehension | |
| マップ（リテラル） | map (literal) | |
| 実行計画 | execution plan / query plan | |
| フィルタのプッシュダウン | filter pushdown | |
| 全文検索 | full-text search | ハイフン付き |
| 形態素解析 | morphological analysis | |
| ファジー検索 / フレーズ検索 | fuzzy search / phrase search | |
| テンポラル型 | temporal types | |

### ストレージ・運用

| 日本語 | 英語 | 備考 |
|---|---|---|
| 永続化 | persistence | |
| 先行書き込みログ（WAL） | write-ahead log (WAL) | |
| グループコミット | group commit | |
| スナップショット | snapshot | |
| トランザクション | transaction | |
| 増分バックアップ | incremental backup | |
| インデックス / プロパティインデックス | index / property index | 複数形 indexes |
| 制約 / 一意制約 / 複合一意制約 | constraint / uniqueness constraint / composite uniqueness constraint | `UNIQUE` はコード扱い |
| リーダー / フォロワー | leader / follower | |
| フェイルオーバー / 昇格 | failover / promotion | `promote-to-leader` |
| レプリケーション | replication | |
| シャーディング / シャード | sharding / shard | |
| ハートビート | heartbeat | |
| 認証 / 認可 | authentication / authorization | |
| ロール | role | admin / writer / reader はコード扱い |
| アクセス制御 | access control | |
| メトリクス / ヘルスチェック | metrics / health check | |
| 監査ログ | audit log | |
| 証明書のホットリロード | certificate hot reload | |
| 対話モード | interactive mode | REPL |
