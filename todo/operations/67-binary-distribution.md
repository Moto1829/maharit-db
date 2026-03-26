# Task 67: バイナリ配布パイプライン

## 概要
`maharit` バイナリを各種プラットフォーム・パッケージマネージャーで配布できるようにする。

## 現状
- `cargo install --path crates/maharit-server` でインストール可能
- ビルド済みバイナリの配布はなし

## 実装内容

### フェーズ1: GitHub Actions リリースパイプライン
- [x] `.github/workflows/release.yml` を作成
  - `v*` タグ push 時にトリガー
  - ターゲット: `x86_64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`
  - `cross` または GitHub-hosted runners でクロスコンパイル
  - `cargo build --release -p maharit-server` でバイナリ生成
  - GitHub Release に自動アップロード（`softprops/action-gh-release`）
  - チェックサム（SHA256）ファイルも添付

### フェーズ2: インストールスクリプト
- [x] `install.sh`（Unix / macOS / Linux 向け）を作成
  - OS/アーキテクチャを自動検出（x86_64/aarch64, Linux/macOS）
  - GitHub Releases から最新バイナリをダウンロード（curl/wget 対応）
  - SHA256 チェックサム検証（sha256sum/shasum 対応）
  - `--version` / `--install-dir` / `--no-confirm` オプション対応
- [x] `install.ps1`（Windows PowerShell 向け）を作成
  - `irm ... | iex` パターンで利用可能
  - SHA256 チェックサム検証（Get-FileHash）
  - ユーザー PATH への自動追加

### フェーズ3: Homebrew
- [ ] Homebrew formula を作成（`maharit-db` tap or homebrew-core）
  - `Formula/maharit.rb` に SHA256 + URL を定義
  - `brew install maharit-db/tap/maharit` でインストール可能に

### フェーズ4: その他パッケージマネージャー
- [ ] `crates.io` への publish（`maharit-server` クレート）
  - `cargo install maharit-server` でインストール可能に
- [ ] AUR（Arch Linux）パッケージ
- [ ] `.deb` / `.rpm` パッケージ生成（`cargo-deb`, `cargo-generate-rpm`）

## 対象ファイル
- `.github/workflows/release.yml`（新規作成）
- `install.sh`（新規作成）
- `install.ps1`（新規作成）
- `Formula/maharit.rb`（Homebrew tap、別リポジトリ）
