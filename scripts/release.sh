#!/usr/bin/env bash
# MaharitDB リリース準備スクリプト
#
# 使い方:
#   bash scripts/release.sh <VERSION|auto> [--push] [--dry-run]
#
#   VERSION   0.3.0 / v0.3.0 / 0.3.0-rc.1 のような SemVer。
#             "auto" なら Conventional Commits から git-cliff が次バージョンを算出
#             (feat → MINOR, fix/perf → PATCH, BREAKING → MAJOR。0.x では BREAKING も MINOR)
#   --push    コミットとタグを origin へ push する（push すると release.yml が走り
#             バイナリ付き GitHub Release が作られる）
#   --dry-run 何も変更せず、次バージョンとリリースノートのプレビューだけ表示
#
# 処理内容:
#   1. 事前チェック（main ブランチ / ワークツリーがクリーン / タグ未使用 / 前バージョンより大きい）
#   2. ルート Cargo.toml の [workspace.package] version を更新し Cargo.lock を同期
#   3. git-cliff で CHANGELOG.md を再生成
#   4. "chore(release): vX.Y.Z" でコミットし、注釈付きタグ vX.Y.Z を作成
#
# 必要ツール: git, cargo, git-cliff (brew install git-cliff / cargo install git-cliff)

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

die() { echo "[ERROR] $*" >&2; exit 1; }
info() { echo "[INFO]  $*"; }

VERSION_ARG=""
PUSH=false
DRY_RUN=false
for a in "$@"; do
  case "$a" in
    --push) PUSH=true ;;
    --dry-run) DRY_RUN=true ;;
    -h|--help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*) die "未知のオプション: ${a}" ;;
    *) [ -z "$VERSION_ARG" ] || die "バージョンが複数指定されています"; VERSION_ARG="$a" ;;
  esac
done
[ -n "$VERSION_ARG" ] || die "バージョンを指定してください（例: bash scripts/release.sh 0.3.0）"

command -v git-cliff >/dev/null || die "git-cliff が必要です: brew install git-cliff"

SEMVER_RE='^[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta|rc)\.[1-9][0-9]*)?$'

current_version() {
  awk '/^\[workspace.package\]/{f=1;next} /^\[/{f=0} f && /^version *=/{gsub(/"/,"",$3); print $3; exit}' Cargo.toml
}
CURRENT="$(current_version)"
[ -n "$CURRENT" ] || die "Cargo.toml から現在のバージョンを読み取れません"

if [ "$VERSION_ARG" = "auto" ]; then
  VERSION="$(git cliff --bumped-version 2>/dev/null)"
  VERSION="${VERSION#v}"
else
  VERSION="${VERSION_ARG#v}"
fi
[[ "$VERSION" =~ $SEMVER_RE ]] || die "SemVer 形式ではありません: ${VERSION}（例: 0.3.0, 0.3.0-rc.1）"
TAG="v${VERSION}"

LAST_TAG="$(git -c versionsort.suffix=- tag --list 'v*' --sort=-v:refname | head -1)"
info "現在の Cargo.toml version : ${CURRENT}"
info "最新タグ                  : ${LAST_TAG:-(なし)}"
info "リリースするバージョン    : ${TAG}"

# 前回タグより大きいこと（sort -V はプレリリースを正しく扱えないので、本体部分で比較）
if [ -n "$LAST_TAG" ]; then
  last_core="${LAST_TAG#v}"; last_core="${last_core%%-*}"
  new_core="${VERSION%%-*}"
  highest="$(printf '%s\n%s\n' "$last_core" "$new_core" | sort -V | tail -1)"
  if [ "$highest" != "$new_core" ]; then
    die "${TAG} は最新タグ ${LAST_TAG} より小さいバージョンです"
  fi
  # 同じ本体バージョンは「前回がプレリリース」のときだけ許可（0.3.0-rc.1 → 0.3.0-rc.2 / 0.3.0）
  if [ "$last_core" = "$new_core" ] && [[ "$LAST_TAG" != *-* ]]; then
    die "${TAG} は最新タグ ${LAST_TAG} 以下です"
  fi
fi
git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null && die "タグ ${TAG} は既に存在します（タグの付け直しは禁止。新しいバージョンを切ってください）"

# CHANGELOG 生成オプションとリリースノートの範囲
#   正式版: 途中の alpha/beta/rc タグを畳み込み、直前の「正式版」タグからの差分にする
#           (git-cliff の --unreleased/--latest は --ignore-tags と併用すると範囲が狭まるので明示指定)
#   プレリリース: 直前のタグ（rc 含む）からの差分
CLIFF_OPTS=()
NOTES_RANGE=(--unreleased)
if [[ "$VERSION" != *-* ]]; then
  CLIFF_OPTS+=('--ignore-tags=-(alpha|beta|rc)\.')
  PREV_FULL="$(git -c versionsort.suffix=- tag --list 'v*' --sort=-v:refname | grep -v -e '-' | head -1 || true)"
  [ -n "$PREV_FULL" ] && NOTES_RANGE=("${PREV_FULL}..HEAD")
fi

if [ "$DRY_RUN" = true ]; then
  echo
  echo "===== リリースノート プレビュー (${TAG}) ====="
  git cliff "${CLIFF_OPTS[@]}" "${NOTES_RANGE[@]}" --tag "$TAG" --strip all 2>/dev/null
  exit 0
fi

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[ "$BRANCH" = "main" ] || die "main ブランチで実行してください（現在: ${BRANCH}）"
[ -z "$(git status --porcelain --untracked-files=no)" ] || die "ワークツリーに未コミットの変更があります"

# 1. バージョン更新（[workspace.package] セクション内の version 行のみ）
awk -v v="$VERSION" '
  /^\[workspace.package\]/{f=1; print; next}
  /^\[/{f=0}
  f && /^version *=/ && !done {print "version = \"" v "\""; done=1; next}
  {print}
' Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml
[ "$(current_version)" = "$VERSION" ] || die "Cargo.toml の更新に失敗しました"
cargo update --workspace --offline >/dev/null 2>&1 || cargo update --workspace >/dev/null

# 2. CHANGELOG 再生成
git cliff "${CLIFF_OPTS[@]}" --tag "$TAG" -o CHANGELOG.md 2>/dev/null

# 3. コミット + タグ
NOTES="$(git cliff "${CLIFF_OPTS[@]}" "${NOTES_RANGE[@]}" --tag "$TAG" --strip all 2>/dev/null)"
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -q -m "chore(release): ${TAG}"
git tag -a --cleanup=verbatim "$TAG" -m "Release ${TAG}" -m "$NOTES"
info "コミットとタグ ${TAG} を作成しました"

if [ "$PUSH" = true ]; then
  git push origin main
  git push origin "$TAG"
  info "push 完了。GitHub Actions (release.yml) がビルドと GitHub Release 作成を行います"
else
  echo
  echo "次のコマンドで公開します:"
  echo "  git push origin main && git push origin ${TAG}"
fi
