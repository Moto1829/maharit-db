# Task 128: セマンティックバージョニングとリリースノートの自動化

状態: 完了 (2026-11-03)

## 背景

バージョニング規則（SemVer + Conventional Commits）は決まっていたが、
CHANGELOG・リリースノート・配布バイナリの作成が手作業で、`v0.1.0` 以降リリースしていなかった。
`install.sh` / `install.ps1` は GitHub Release のアセットを前提にしているのに、それを作る仕組みが無かった。
また `maharit --version` を実行すると REPL が起動してしまい、install.sh のバージョン表示が機能していなかった。

## 実施内容

- [x] `cliff.toml`: git-cliff 設定。Conventional Commits を日本語カテゴリ（新機能 / バグ修正 / パフォーマンス / …）に分類
  - `todo:` / `docs(todo)` / `chore(todo)` / `style:` / `chore(release)` はノートから除外
  - `(#NN)` はタスク番号なので PR リンク化しない
  - `[bump]`: 0.x の間は BREAKING でも MINOR（`breaking_always_bump_major = false`）
- [x] `CHANGELOG.md`: 既存履歴から生成（Keep a Changelog 形式、`[Unreleased]` + `0.1.0`）
- [x] `scripts/release.sh <VERSION|auto> [--push] [--dry-run]`
  - SemVer 形式・前回タグより大きいこと・タグ未使用・main/クリーンを検査
  - `[workspace.package] version` 更新 → `Cargo.lock` 同期 → CHANGELOG 再生成 → `chore(release): vX.Y.Z` コミット → 注釈付きタグ（本文にリリースノート）
  - 正式版は途中の alpha/beta/rc を畳み込む（直前の正式版からの差分）
- [x] `.github/workflows/release.yml`（タグ `v*.*.*` push で起動）
  - タグと Cargo.toml の version の一致・SemVer 形式を検証
  - git-cliff でリリースノート生成 + インストール手順を追記
  - 5 ターゲット（linux x86_64/aarch64, macOS x86_64/aarch64, windows x86_64）をビルドし、
    install.sh / install.ps1 が期待する `maharit-<tag>-<target>.tar.gz|.zip` と `.sha256` を作成
  - `-rc.N` 等は `--prerelease`、正式版は `--latest` で GitHub Release 作成
  - `workflow_dispatch` ではビルドとノート生成のみ（タグ前の事前確認用）
- [x] `maharit --version` / `-V` / `version` を追加（REPL が起動していた）

## 検証

- 使い捨てクローンで `scripts/release.sh 0.2.0` を実行し、Cargo.toml・CHANGELOG・タグ本文が正しく生成されることを確認
- rc タグを挟んだ場合に、正式版ノートが直前の正式版からの差分になることを確認
- `actionlint` で release.yml を検査
- `cargo clippy -p maharit-server --all-targets -D warnings` / `cargo fmt --check`

## 未実施

- 実際のタグ push による release.yml の実行（Windows / aarch64 Linux ビルドは未検証）。
  初回は Actions の「Release」を `workflow_dispatch` で手動実行して全ターゲットのビルドを確認してからタグを打つこと
