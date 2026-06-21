# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## プロジェクト概要

MaharitDB は Rust で実装された Cypher ライクなグラフデータベースエンジン。`crates/` 配下に 8 個のクレートで構成された Cargo workspace（Rust edition 2024、rustc 1.75+）。バイナリ名は `maharit`（`maharit-server` クレートが生成）。

## ビルド・テスト・実行コマンド

```bash
# Workspace 全体のビルド
cargo build                          # debug
cargo build --release -p maharit-server   # 本番バイナリ（target/release/maharit）

# Workspace 全テスト
cargo test                           # 全クレート
cargo test -p maharit-query          # 単一クレート
cargo test -p maharit-query executor::tests::test_match   # 単一テスト（名前部分一致）
cargo test -- --nocapture            # println! を表示

# Lint / フォーマット
cargo fmt --all
cargo clippy --all-targets -- -D warnings

# REPL（対話モード）
cargo run -p maharit-server

# TCP サーバー起動（デフォルト 127.0.0.1:7687）
cargo run -p maharit-server -- server --host 0.0.0.0 --port 7687 --data /tmp/maharit.db

# サーバーサブコマンド: server / backup / restore / admin
cargo run -p maharit-server -- backup --source ./db --output ./backup.db --compress-zstd
cargo run -p maharit-server -- restore ./backup.db --output ./db
cargo run -p maharit-server -- admin promote-to-leader --addr 127.0.0.1:7689

# サンプル
cargo run --example basic
cargo run --example traversal
```

### E2E テスト（Python スクリプト）

`scripts/` 配下の Python スクリプトは TCP プロトコル（4 バイト長プレフィックス + JSON）で稼働中サーバーに接続して検証する。**事前にサーバーを起動しておく必要がある**。

```bash
# サーバー起動 → 別ターミナルで実行
python3 scripts/smoke_test.py                                # 基本動作確認
python3 scripts/query_feature_test.py                        # クエリ機能網羅
python3 scripts/benchmark.py --nodes 10000                   # 性能ベンチ
python3 scripts/concurrent_test.py
python3 scripts/constraint_test.py
python3 scripts/persistence_test.py
python3 scripts/auth_test.py

# レプリケーション 3 ノードクラスター（リーダー 7687 / フォロワー 7689,7690）
bash scripts/start_replication_local.sh
bash scripts/stop_replication_local.sh
python3 scripts/replication_test.py
python3 scripts/failover_test.py
```

ベンチ結果は `benchmark_reports/bench_<timestamp>.md` に保存される。`benchmark_reports/baseline.json` が基準値。ベンチ実行後は `analyze-benchmark` スキルでレポートを分析できる。

## アーキテクチャ

### クレート依存関係

```
maharit-server  ──┬─→ maharit-query   ─→ maharit-core
                  ├─→ maharit-storage  ─→ maharit-core
                  └─→ maharit-cluster  ─→ maharit-core
maharit-io      ──→ maharit-core
maharit-viz     ──┬─→ maharit-core
                  └─→ maharit-client
maharit-client（独立: tokio TCP クライアント。maharit-core にも依存しない）
```

### maharit-query: クエリパイプライン

クエリ実行は **lexer → parser → planner → executor** の 4 段構成。新しいクエリ句や関数を追加する際は、この 4 ファイルすべてに変更が必要。

- `ast.rs`: `Statement` enum がトップレベル文（Create / Match / Merge / Unwind / Foreach / CreateConstraint / CreateFulltextIndex / Explain / Profile 等）。`Expression` enum は値計算（ScalarFunction / BinaryOp / Property / Map / ListPredicate / ExistsSubquery 等）。`is_read_only(&Statement)` で読み取り専用判定。
- `lexer.rs`: トークンベース。キーワードは大文字小文字を区別しない。
- `parser.rs`: 再帰下降。`parse()` は最初のトークンで分岐するが、`CREATE CONSTRAINT` / `CREATE FULLTEXT INDEX` は peek-ahead で識別。
- `planner.rs`: `QueryPlan` / `PlanNode` を構築。EXPLAIN / PROFILE で使用。`GraphStats` を元にフィルタプッシュダウン・インデックス選択・JOIN 順序・カラムプルーニングを行う。
- `executor.rs`: `Executor<'a>` は `*mut Graph`（生ポインタ）+ `readonly: bool` + `ConstraintManager` + `FulltextManager` を保持。`graph_ref()` / `graph_mut()` でアクセス。読み取り並列実行のため `unsafe new_readonly(&Graph)` と `unsafe impl Sync` を持つ。Bindings は `HashMap<String, BindingValue>` で `BindingValue = Node | Edge | Path | Scalar`。
- `cache.rs`: `AstCache` / `PlanCache` / `QueryCache`。PlanCache は LRU + 統計スナップショット（node_count / edge_count）による無効化。

**新ステートメント追加の典型手順**: `Statement` enum 追加 → AST 型追加 → `lexer.rs` にキーワード登録 → `parser.rs` にパーサーメソッド追加 → `executor.rs` に実行メソッド追加 → `planner.rs` に case 追加。

### maharit-core: グラフエンジン

- `graph.rs`: `Graph` / `Node` / `Edge` / `NodeId` / `EdgeId`。隣接リストは `Vec<HashSet<EdgeId>>`（O(1) 削除のため Vec から変更済み）。`Node.properties` / `Edge.properties` は `Arc<HashMap<>>`（結果取得時のクローン削減）。
- `concurrent_graph.rs`: `ConcurrentGraph`（DashMap ベース、並行読み書き向け）。
- `graph_backend.rs`: `GraphBackend` トレイトで `Graph` と `ConcurrentGraph` を抽象化。
- `traversal.rs`: `Dijkstra` / `AStar` / `Traversal` / `Path` / `all_paths(graph, from, to, max_depth)`（DFS バックトラック）。`Dijkstra::distances_from` は dense `Vec` で高速化。
- `algorithms.rs`: PageRank、媒介中心性、近接中心性、強連結成分、ラベル伝搬、サイクル検出、トポロジカルソート。rayon で並列化済み。
- `fulltext.rs`: BM25 全文検索。IDF は `ln(1 + (N-df+0.5)/(df+0.5))` 変種（負スコア回避）。`FulltextManager` で複数インデックス管理。rayon で並列トークン化。
- `constraint.rs`: UNIQUE / NOT NULL / 型チェック / 複合 UNIQUE。`Constraint.properties: Vec<String>`（複合対応）。
- `property_index.rs`: B-tree ベースのプロパティインデックス。`indexed_properties: HashSet<(String, String)>` を `GraphStats` 経由でプランナに渡す。

### maharit-storage: 永続化

- `wal.rs` / `wal_group_commit.rs`: Write-Ahead Log。グループコミットでスループット向上。
- `persistence.rs`: `PersistentStorage::load(path)` / `save(path)` でファイル全体を bincode で保存・復元。
- `mvcc.rs`: MVCC トランザクション。
- `backup.rs`: `Backup::create` / `restore` / `verify` / `metadata`。`CompressionType::None | Gzip | Zstd`。

### maharit-server: TCP / HTTP / 認証

- `main.rs`: CLI のエントリーポイント。サブコマンドなしで REPL、`server` で TCP サーバー、`backup` / `restore` / `admin` あり。
- `tcp_server.rs`: 4 バイト長プレフィックス + JSON プロトコル。`streamQuery` 型は複数レスポンスをストリーミング。
- `http_server.rs`: 軽量 tokio TCP。`/metrics`（Prometheus）、`/health`、`/health/live`、`/health/ready`。
- `auth.rs`: ユーザー・セッション管理、RBAC（admin / writer / reader）、`CREATE USER` / `DROP USER` / `ALTER USER` を SQL ライクに処理。
- `tls.rs`: rustls 0.23（証明書ホットリロード対応）。`builder_with_protocol_versions` を使う（`versions` フィールドは private）。
- `replication.rs`: `NodeRole` / `ReplicationConfig` / `LeaderReplicationManager` / `FollowerReplicationManager`。tokio `broadcast::channel` で WAL ストリーミング、ハートビートで生存検知。
- `coordinator.rs`: シャーディングコーディネーター（`maharit-cluster` の Strategy / Router を利用）。
- `tracing_setup.rs` / `metrics.rs` / `audit.rs` / `logging.rs`: OpenTelemetry トレーシング + Prometheus + 監査ログ + JSON 構造化ログ。

### maharit-cluster: シャーディング

`coordinator.rs` / `router.rs` / `shard.rs` / `shard_client.rs` / `strategy.rs`。Hash / Range シャーディング戦略。`ClusterConfig` で `shards: Vec<ShardConfig>` を保持。

## プロジェクトルール（CLAUDE.local.md より）

- **タスクは `todo/` 配下にカテゴリフォルダ分けで作成する**: `query-core/`, `query-clauses/`, `query-functions/`, `graph/`, `storage/`, `server/`, `client/`, `operations/`, `e2e/`, `bug/`, `infra/`, `docs/`。完了タスクは `todo-archive/` へ移動。
- **タスクごとに git commit + git push を行う**: 1 タスク = 1 コミットを徹底。
- `reference/` ディレクトリはユーザー指示がない限り参照しない。
- プロジェクトルール・規約は `docs/` ではなく `~/.claude/projects/<encoded>/memory/` に保存する方針（過去のフィードバック）。

## バージョニング規約

- セマンティックバージョニング。現在 `0.x.y`（unstable）。
- バージョンはルート `Cargo.toml` の `[workspace.package] version` でのみ管理（workspace 継承）。
- リリース手順: version bump → CHANGELOG 更新 → commit → tag（`v0.2.0` 形式、`v` プレフィックス必須）→ push → `gh release create`。
- Conventional Commits: `feat` → MINOR、`fix` / `perf` → PATCH、`feat!` または `BREAKING CHANGE` → MAJOR。
- タグの force-push は禁止。修正が必要な場合は新バージョンを切る。

## Rust 2024 edition の注意点

- `extern "C"` ブロックは `unsafe extern "C"` と書く。
- `std::env::set_var` / `remove_var` は `unsafe` ブロック内でのみ呼べる。
