# バイナリ配布パイプライン

## 概要
`maharit` バイナリを各種プラットフォーム・パッケージマネージャーで配布できるようにする。

## 現状
- `cargo install --path crates/maharit-server` でインストール可能
- ビルド済みバイナリの配布はなし

## 実装内容

### フェーズ1: GitHub Actions リリースパイプライン
- [ ] `.github/workflows/release.yml` を作成
  - `v*` タグ push 時にトリガー
  - ターゲット: `x86_64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`
  - `cross` または GitHub-hosted runners でクロスコンパイル
  - `cargo build --release -p maharit-server` でバイナリ生成
  - GitHub Release に自動アップロード（`softprops/action-gh-release`）
  - チェックサム（SHA256）ファイルも添付

### フェーズ2: インストールスクリプト
- [ ] `install.sh`（Unix 向け）を作成
  - OS/アーキテクチャを自動検出
  - GitHub Releases から最新バイナリをダウンロード
  - `/usr/local/bin/maharit` に配置
- [ ] `install.ps1`（Windows 向け）を作成

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
