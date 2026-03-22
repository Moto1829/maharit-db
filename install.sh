#!/usr/bin/env bash
# MaharitDB インストールスクリプト（Unix / macOS / Linux 向け）
#
# 使い方:
#   curl -fsSL https://raw.githubusercontent.com/Moto1829/maharit-db/main/install.sh | bash
#   または
#   bash install.sh [OPTIONS]
#
# オプション:
#   --version <VERSION>   インストールするバージョン（例: v0.2.0）。省略時は最新
#   --install-dir <DIR>   インストール先ディレクトリ（デフォルト: /usr/local/bin）
#   --no-confirm          確認プロンプトをスキップ
#   --help                このメッセージを表示

set -euo pipefail

REPO="Moto1829/maharit-db"
BINARY_NAME="maharit"
DEFAULT_INSTALL_DIR="/usr/local/bin"

# ── カラー出力 ──────────────────────────────────────────────────────────────
if [ -t 1 ]; then
  RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
  BLUE='\033[0;34m'; BOLD='\033[1m'; RESET='\033[0m'
else
  RED=''; GREEN=''; YELLOW=''; BLUE=''; BOLD=''; RESET=''
fi

info()    { echo -e "${BLUE}[INFO]${RESET}  $*"; }
success() { echo -e "${GREEN}[OK]${RESET}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${RESET}  $*"; }
error()   { echo -e "${RED}[ERROR]${RESET} $*" >&2; exit 1; }

# ── 引数解析 ────────────────────────────────────────────────────────────────
VERSION=""
INSTALL_DIR="$DEFAULT_INSTALL_DIR"
NO_CONFIRM=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)    VERSION="$2"; shift 2 ;;
    --install-dir) INSTALL_DIR="$2"; shift 2 ;;
    --no-confirm) NO_CONFIRM=true; shift ;;
    --help)
      grep '^#' "$0" | head -20 | sed 's/^# \?//'
      exit 0
      ;;
    *) error "未知のオプション: $1" ;;
  esac
done

# ── OS / アーキテクチャ検出 ─────────────────────────────────────────────────
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)  OS_ID="linux" ;;
  Darwin) OS_ID="darwin" ;;
  *) error "非対応 OS: $OS" ;;
esac

case "$ARCH" in
  x86_64)          ARCH_ID="x86_64" ;;
  aarch64 | arm64) ARCH_ID="aarch64" ;;
  *) error "非対応アーキテクチャ: $ARCH" ;;
esac

TARGET="${ARCH_ID}-unknown-${OS_ID}-gnu"
if [ "$OS_ID" = "darwin" ]; then
  TARGET="${ARCH_ID}-apple-darwin"
fi

# ── 最新バージョン取得 ───────────────────────────────────────────────────────
if [ -z "$VERSION" ]; then
  info "最新バージョンを確認中..."
  if command -v curl &>/dev/null; then
    VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
      | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": "\(.*\)".*/\1/')"
  elif command -v wget &>/dev/null; then
    VERSION="$(wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" \
      | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": "\(.*\)".*/\1/')"
  else
    error "curl または wget が必要です"
  fi
  [ -z "$VERSION" ] && error "最新バージョンを取得できませんでした"
fi

# v プレフィックス正規化
[[ "$VERSION" != v* ]] && VERSION="v${VERSION}"

ARCHIVE_NAME="${BINARY_NAME}-${VERSION}-${TARGET}.tar.gz"
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE_NAME}"
CHECKSUM_URL="${DOWNLOAD_URL%.tar.gz}.sha256"

echo ""
echo -e "${BOLD}MaharitDB インストーラー${RESET}"
echo "  バージョン  : ${VERSION}"
echo "  ターゲット  : ${TARGET}"
echo "  インストール先: ${INSTALL_DIR}/${BINARY_NAME}"
echo ""

if [ "$NO_CONFIRM" = false ]; then
  read -r -p "インストールを続行しますか？ [y/N] " ans
  [[ "$ans" =~ ^[yY] ]] || { info "キャンセルしました"; exit 0; }
fi

# ── ダウンロード & 検証 ──────────────────────────────────────────────────────
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

ARCHIVE_PATH="${TMP_DIR}/${ARCHIVE_NAME}"

info "ダウンロード中: ${DOWNLOAD_URL}"
if command -v curl &>/dev/null; then
  curl -fsSL --progress-bar -o "$ARCHIVE_PATH" "$DOWNLOAD_URL" \
    || error "ダウンロードに失敗しました"
else
  wget -q --show-progress -O "$ARCHIVE_PATH" "$DOWNLOAD_URL" \
    || error "ダウンロードに失敗しました"
fi

# SHA256 チェックサム検証（ファイルが存在する場合のみ）
if command -v curl &>/dev/null; then
  if curl -fsSL "$CHECKSUM_URL" -o "${TMP_DIR}/checksum.sha256" 2>/dev/null; then
    info "チェックサムを検証中..."
    cd "$TMP_DIR"
    if command -v sha256sum &>/dev/null; then
      sha256sum -c checksum.sha256 --status \
        || error "チェックサムの検証に失敗しました。ファイルが破損している可能性があります"
    elif command -v shasum &>/dev/null; then
      shasum -a 256 -c checksum.sha256 --status \
        || error "チェックサムの検証に失敗しました。ファイルが破損している可能性があります"
    else
      warn "sha256sum / shasum が見つかりません。チェックサム検証をスキップします"
    fi
    cd - >/dev/null
    success "チェックサム検証 OK"
  else
    warn "チェックサムファイルが見つかりません。スキップします"
  fi
fi

# ── 展開 & インストール ──────────────────────────────────────────────────────
info "展開中..."
tar -xzf "$ARCHIVE_PATH" -C "$TMP_DIR"

BINARY_PATH="${TMP_DIR}/${BINARY_NAME}"
[ -f "$BINARY_PATH" ] || error "アーカイブにバイナリが見つかりません: ${BINARY_NAME}"

chmod +x "$BINARY_PATH"

# インストール先ディレクトリが書き込み可能か確認
if [ -w "$INSTALL_DIR" ]; then
  mv "$BINARY_PATH" "${INSTALL_DIR}/${BINARY_NAME}"
else
  info "管理者権限が必要です (sudo)"
  sudo mv "$BINARY_PATH" "${INSTALL_DIR}/${BINARY_NAME}"
fi

# ── 確認 ────────────────────────────────────────────────────────────────────
if command -v "${BINARY_NAME}" &>/dev/null; then
  INSTALLED_VERSION="$("${BINARY_NAME}" --version 2>/dev/null || true)"
  success "インストール完了: ${INSTALL_DIR}/${BINARY_NAME}"
  [ -n "$INSTALLED_VERSION" ] && echo "  ${INSTALLED_VERSION}"
else
  warn "${INSTALL_DIR} が PATH に含まれていません"
  echo "  以下を ~/.bashrc や ~/.zshrc に追加してください:"
  echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
fi

echo ""
echo -e "${GREEN}MaharitDB ${VERSION} のインストールが完了しました！${RESET}"
echo "  使い方: maharit --help"
