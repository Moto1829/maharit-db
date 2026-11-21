# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## プロジェクト概要

MaharitDB は Rust で実装された Cypher ライクなグラフデータベースエンジン。`crates/` 配下の 8 クレートの Cargo workspace（edition 2024。Rust の版は `rust-toolchain.toml` で固定）。バイナリ名は `maharit`（`maharit-server` が生成）。
クレート固有の約束事は各クレートの `CLAUDE.md`（`crates/maharit-{query,server,storage,core}/CLAUDE.md`）にある。

```
maharit-server  ──┬─→ maharit-query   ─→ maharit-core
                  ├─→ maharit-storage  ─→ maharit-core
                  └─→ maharit-cluster  ─→ maharit-core
maharit-io  ─→ maharit-core      maharit-viz ─→ maharit-core, maharit-client
maharit-client（独立の tokio TCP クライアント）
```

## 検証・ビルド・実行

**コミット前は `bash scripts/check.sh`**（fmt → clippy → test → docs の翻訳チェック。CI の lint / test と同じ検査）。
最後の行が `CHECK-OK` か `CHECK-FAILED: <段>`。`--quick` は origin/main から変更のあったクレートとその依存元だけを検査する。
コミットとつなぐときは `bash scripts/check.sh > check.log 2>&1 && git commit …`（パイプで `tail` に渡すと失敗しても先へ進む）。

```bash
cargo build --release -p maharit-server                  # target/release/maharit
cargo test -p maharit-query executor::tests::read_basic  # 名前の一部でテストを絞る
cargo clippy --workspace --all-targets -- -D warnings    # --workspace 必須（無いとルートパッケージだけ）
cargo run -p maharit-server                              # REPL
cargo run -p maharit-server -- server --port 7687 --data /tmp/maharit.db   # TCP サーバー
```

lint は workspace 共通（ルート `Cargo.toml` の `[workspace.lints]`）。`unsafe` には `// SAFETY:` が必須。

### CI

- `.github/workflows/test.yml`: `scripts/check.sh --stage lint|test`、soak（乱数テストの追加シード）、E2E、python-native。docs・todo・`*.md` だけの変更では動かない
- Rust の版を上げるときはワークフローの `dtolnay/rust-toolchain@<版>` も同時に更新する
- Linux 専用コード（`cfg(target_os = "linux")`）は macOS の clippy では検査されない
- `audit.yml`: 依存の脆弱性検査（cargo-audit）。`dependabot.yml`: 週次の更新 PR

### E2E とベンチマーク

- E2E 一式: `bash scripts/run_e2e_local.sh`（要 release ビルド、Docker 不要）。個別のスクリプト（`scripts/*_test.py`）を手元のサーバーに向けるときは `MAHARIT_E2E_LOCAL=1`。レプリケーションは `scripts/start_replication_local.sh`（7687 / 7689 / 7690）
- ベンチ: `bash scripts/run_bench_local.sh --name before`（エンジン単体 + サーバー）、`cargo bench -p maharit-query --bench engine -- read/hop`（項目を絞る）。前後の比較は `python3 scripts/perf_check.py <前>.json <後>.json`。結果は `benchmark_reports/`（git 管理外）。分析はスキル `analyze-benchmark`

## docs

- `docs/foo.md`（日本語）が正本。英語は `docs/foo.en.md`。日本語を変えたら同じコミットで英語も直す（そうすれば `source_commit` は変えなくてよい）。訳語は `docs/_i18n/GLOSSARY.md`
- ```cypher ブロックは `docs_cypher_examples` テストでパースされる。1 段落 1 文、コメントは `//`、出力例は ```text。英語版の ```cypher はコメント以外を日本語版と一致させる。未対応は `crates/maharit-query/tests/docs_cypher_known_failures.txt`
- `mkdocs.yml` の nav を変えたら `nav_translations` も。翻訳の鮮度は `python3 scripts/check_docs_i18n.py`、サイトのビルドは `.venv-docs/bin/mkdocs build --strict`

## プロジェクトルール

- タスクは `todo/<カテゴリ>/` に作り（`query-core/` `query-clauses/` `query-functions/` `graph/` `storage/` `server/` `client/` `operations/` `e2e/` `bug/` `infra/` `docs/`）、完了したら `todo-archive/` へ移す
- 1 タスク = 1 コミットで push する。句・関数の追加、バグ修正、リリースの手順はスキル（`add-cypher-clause` / `add-cypher-function` / `fix-bug` / `release`）
- `reference/` はユーザーの指示がない限り参照しない
- 開発の規約は `docs/` ではなくメモリに置く

## バージョニング・リリース

- [Semantic Versioning 2.0.0](https://semver.org/lang/ja/)（`MAJOR.MINOR.PATCH`）。現在は `0.x.y`（API 未安定）。
- バージョンはルート `Cargo.toml` の `[workspace.package] version` の 1 か所でのみ管理（各クレートは `version.workspace = true`）。`maharit --version` で確認できる。
- バージョンは **リリース時にだけ** 上げる。普段のコミットで `Cargo.toml` の version を触らない（`scripts/release.sh` が更新する）。

### コミットの種類と上げ幅

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

## Rust 2024 edition の注意点

- `extern "C"` ブロックは `unsafe extern "C"` と書く
- `std::env::set_var` / `remove_var` は `unsafe` ブロック内でのみ呼べる
