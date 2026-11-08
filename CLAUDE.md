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
cargo test --workspace               # 全クレート（--workspace 無しはルートパッケージのみ）
cargo test -p maharit-query          # 単一クレート
cargo test -p maharit-query executor::tests::test_match   # 単一テスト（名前部分一致）
cargo test -- --nocapture            # println! を表示

# Lint / フォーマット
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings   # --workspace 必須（無いとルートパッケージしか検査されない）

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

### CI

`.github/workflows/test.yml`: PR / main push で fmt・clippy（`--workspace`）・`cargo test --workspace`・E2E を実行。
Rust は `rust-toolchain.toml` で固定（上げる時はワークフローの `dtolnay/rust-toolchain@<版>` も同時に更新）。
Linux 専用コード（`cfg(target_os = "linux")`）は macOS の clippy では検査されない点に注意。
`.github/workflows/audit.yml`: 依存変更時と毎週月曜に `cargo-audit` で RustSec 勧告を検査（脆弱性で失敗、メンテ終了等は警告）。ローカルでは `cargo install cargo-audit --locked --root target/tools` → `target/tools/bin/cargo-audit audit`。`.github/dependabot.yml` で cargo / github-actions の週次更新 PR。

### docs の Cypher 例

`docs/` の ```cypher ブロックは `cargo test -p maharit-query --test docs_cypher_examples` で全てパース検査される。1 段落 1 ステートメント（空行区切り）、コメントは `//`（`--` は非対応）、出力例や REPL の入出力は ```text ブロックに書く。未対応のまま残す記述は `crates/maharit-query/tests/docs_cypher_known_failures.txt` に載せる。

### docs の英語版（i18n）

- 日本語 `docs/foo.md` が正本。英語は同じフォルダの `docs/foo.en.md`（mkdocs-static-i18n の suffix 構成）。サイトは日本語が `/`、英語が `/en/`。英語版が無いページは英語サイトでも日本語版が表示される。README は `README.md`（英語）/ `README.ja.md`（日本語）。
- 訳語・書式は `docs/_i18n/GLOSSARY.md` に従う（サイトには出ない）。英語ファイルの先頭に `source_commit: <翻訳元の日本語ファイルの最新コミット>` を書く。
- 日本語ページを変更したら、対応する `.en.md` も同じコミットで直す（日英を同じコミットで直した場合、そのコミットは反映済みとみなすので `source_commit` は変えなくてよい。英語だけを後から追従させるときは `source_commit` をその時点の日本語の最新コミットに上げる）。その場で訳せないときは `.en.md` の冒頭に `!!! warning "This translation may be outdated"` を付ける。
- `.en.md` の ```cypher ブロックはコメント（`//`・`/* */`）以外を日本語版と完全に一致させる（`docs_cypher_examples` テストが検査。known-failures は日本語ファイル名のキーで共有）。
- `mkdocs.yml` の nav に見出しを追加・変更したら、`plugins.i18n` の `nav_translations` も更新する。
- ローカル確認: `python3 -m venv .venv-docs && .venv-docs/bin/pip install -r requirements-docs.txt && .venv-docs/bin/mkdocs build --strict`（CI と同じ。PR でも `docs.yml` が `--strict` ビルドする）。

### E2E テスト（Python スクリプト）

CI と同じ E2E 一式は Docker なしで `bash scripts/run_e2e_local.sh`（要 `cargo build --release -p maharit-server`）。
個別に実行する場合、Docker を使わず手元で起動したサーバーに対しては `MAHARIT_E2E_LOCAL=1` を付ける。


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
- `persistence.rs`: `PersistentStorage::load(path)` / `save(path)` でファイル全体を保存・復元（保存は tmp + fsync + rename で原子的）。現行は v4 形式（bincode 1 互換のバイト列を `snapshot_codec.rs` が自前で読み書き。bincode 依存は削除済み）。形式を変えるときは `VERSION` を上げ、旧版の読み込みを残す。`tests/fixtures/v4_*.mgdb` は bincode 時代に書いた互換性確認用ファイル
- `mvcc.rs`: MVCC トランザクション。
- `backup.rs`: `Backup::create` / `restore` / `verify` / `metadata`。`CompressionType::None | Gzip | Zstd`。

### maharit-server: TCP / HTTP / 認証

- `main.rs`: CLI のエントリーポイント。サブコマンドなしで REPL、`server` で TCP サーバー、`backup` / `restore` / `admin` あり。
- `tcp_server.rs`: 4 バイト長プレフィックス + JSON プロトコル。`streamQuery` 型は複数レスポンスをストリーミング。
- `http_server.rs`: 軽量 tokio TCP。`/metrics`（Prometheus）、`/health`、`/health/live`、`/health/ready`。
- `auth.rs`: ユーザー・セッション管理、RBAC（admin / writer / reader）、`CREATE USER` / `DROP USER` / `ALTER USER` を SQL ライクに処理。
- `tls.rs`: rustls 0.23（証明書ホットリロード対応）。`builder_with_protocol_versions` を使う（`versions` フィールドは private）。
- `replication.rs`: `NodeRole` / `ReplicationConfig` / `LeaderReplicationManager` / `FollowerReplicationManager`。tokio `broadcast::channel` で WAL ストリーミング、ハートビートで生存検知。
- `durable_wal.rs`: サーバーのディスク WAL（`<data>.wal`）。書き込みクエリの `WalEntryData`（レプリケーションと同じ変更ログ、`mutation_log.rs` の `RecordingGraph` が記録）を書き込みゲート内で追記し、ゲート外で fsync（グループコミット）してから応答。起動時はスナップショット読み込み → WAL 再生（`replication::apply_wal_entry`）。チェックポイント（停止時・`--checkpoint-mb` 超過時、`tcp_server::Checkpointer`）はゲートを取ってスナップショット保存 → WAL を空にする。WAL ヘッダーのスナップショット SHA-256 が一致しない WAL は捨てる。WAL は ID で対象を指すので、`load_concurrent` はノード・エッジ ID を保存時のまま復元する（振り直さないこと）。`--wal fsync|os|off`、フォロワーは WAL なし
- `coordinator.rs`: シャーディングコーディネーター（`maharit-cluster` の Strategy / Router を利用）。
- `tracing_setup.rs` / `metrics.rs` / `audit.rs` / `logging.rs`: OpenTelemetry トレーシング + Prometheus + 監査ログ + JSON 構造化ログ。

### maharit-cluster: シャーディング

`coordinator.rs` / `router.rs` / `shard.rs` / `shard_client.rs` / `strategy.rs`。Hash / Range シャーディング戦略。`ClusterConfig` で `shards: Vec<ShardConfig>` を保持。

## プロジェクトルール（CLAUDE.local.md より）

- **タスクは `todo/` 配下にカテゴリフォルダ分けで作成する**: `query-core/`, `query-clauses/`, `query-functions/`, `graph/`, `storage/`, `server/`, `client/`, `operations/`, `e2e/`, `bug/`, `infra/`, `docs/`。完了タスクは `todo-archive/` へ移動。
- **タスクごとに git commit + git push を行う**: 1 タスク = 1 コミットを徹底。
- `reference/` ディレクトリはユーザー指示がない限り参照しない。
- プロジェクトルール・規約は `docs/` ではなく `~/.claude/projects/<encoded>/memory/` に保存する方針（過去のフィードバック）。

## バージョニング・リリース

- [Semantic Versioning 2.0.0](https://semver.org/lang/ja/)（`MAJOR.MINOR.PATCH`）。現在は `0.x.y`（API 未安定）。
- バージョンはルート `Cargo.toml` の `[workspace.package] version` の 1 か所でのみ管理（各クレートは `version.workspace = true`）。`maharit --version` で確認できる。
- バージョンは **リリース時にだけ** 上げる。普段のコミットで `Cargo.toml` の version を触らない（`scripts/release.sh` が更新する）。

### バージョンのインクリメント規則

前回リリースから main に入ったコミットのうち、**最も影響の大きいもの 1 つ**で上げ幅が決まる（`bash scripts/release.sh auto` が `cliff.toml` の `[bump]` に従って自動算出）。上げた桁より下の桁は 0 に戻す。

| 前回リリース以降の変更 | コミット種別 | `0.x.y` の間（現在） | `1.0.0` 以降 |
|---|---|---|---|
| 後方互換を壊す変更 | `feat!:` / `fix!:` / フッター `BREAKING CHANGE:` | MINOR（0.2.3 → 0.3.0） | MAJOR（1.4.2 → 2.0.0） |
| 後方互換のある機能追加 | `feat:` | MINOR（0.2.3 → 0.3.0） | MINOR（1.4.2 → 1.5.0） |
| バグ修正・性能改善・セキュリティ修正 | `fix:` / `perf:` / `security:` | PATCH（0.2.3 → 0.2.4） | PATCH（1.4.2 → 1.4.3） |
| 利用者に影響しない変更のみ | `docs:` / `test:` / `refactor:` / `chore:` / `ci:` / `build:` / `bench:` / `style:` / `todo:` | リリースしない（必要なら PATCH） | 同左 |

**後方互換を壊す変更**（`!` を付ける）とみなすもの:
- TCP プロトコル（4 バイト長プレフィックス + JSON）のリクエスト・レスポンス形式の非互換変更
- 永続化ファイル・WAL・バックアップのフォーマット変更で、旧バージョンのデータを読めなくなるもの
- 既存の Cypher 構文・関数の削除や、同じクエリで結果が変わる意味の変更
- CLI のサブコマンド・フラグの削除や改名、デフォルト値の変更で既存の起動方法が動かなくなるもの
- `maharit-client` / Python クライアントなど公開 API の削除・シグネチャ変更
- 認証・RBAC・レプリケーション設定の非互換変更（既存クラスタやユーザー設定がそのままでは動かない）

互換を保つ変更（新しい構文・関数・フラグ・HTTP エンドポイントの追加、省略可能なフィールドの追加）は `feat:`。

**プレリリース**: `X.Y.Z-alpha.N` → `-beta.N` → `-rc.N` の順。N は 1 から数え、同じ段階の中で 1 ずつ上げる（`0.3.0-rc.1` → `0.3.0-rc.2` → `0.3.0`）。本体部分 `X.Y.Z` には、正式版で出す予定のバージョンを使う。

**1.0.0 への移行**: 公開 API・プロトコル・データフォーマットを安定版として約束する時点で `1.0.0` にする。その際 `cliff.toml` の `breaking_always_bump_major` を `true` に変える。

**その他**:
- 一度打ったタグは削除・force-push しない。リリースに問題があれば、修正して次の PATCH を出す。
- 判断に迷ったら大きい方を選ぶ（壊れるかもしれない変更には `!` を付ける）。
- コミットメッセージがそのまま CHANGELOG / リリースノートになるので、利用者に分かる言葉で書く（`todo:` / `docs(todo)` / `chore(todo)` / `style:` はノートに載らない）。
- タグ名は `vX.Y.Z` / `vX.Y.Z-rc.N`（`v` を必ず付ける）。

### リリース手順

```bash
brew install git-cliff                        # 初回のみ
bash scripts/release.sh auto --dry-run        # 次バージョンとリリースノートのプレビュー
bash scripts/release.sh 0.3.0                 # version 更新 → CHANGELOG 再生成 → commit → tag
bash scripts/release.sh 0.3.0 --push          # 上記 + push（release.yml が起動）
git cliff -o CHANGELOG.md                     # [Unreleased] を含めて CHANGELOG だけ再生成
```

- タグ push で `.github/workflows/release.yml` が起動: タグと Cargo.toml の version 一致を検証 → 5 ターゲットをビルド → `maharit-<tag>-<target>.tar.gz|.zip` + `.sha256`（`install.sh` / `install.ps1` が参照する名前）→ git-cliff のノートで GitHub Release 作成（`-rc.N` 等は prerelease）。
- 正式版のノートは途中の rc を畳み込み、直前の正式版からの差分になる。
- タグ前に全ターゲットのビルドを確かめたいときは Actions の Release を手動実行（`workflow_dispatch` はビルドのみで Release は作らない）。

## Rust 2024 edition の注意点

- `extern "C"` ブロックは `unsafe extern "C"` と書く。
- `std::env::set_var` / `remove_var` は `unsafe` ブロック内でのみ呼べる。
