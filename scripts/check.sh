#!/usr/bin/env bash
# コミット前の検証を 1 本で回す（CI の test.yml の lint / test と同じ検査）。
#
#   bash scripts/check.sh            # 全部: fmt → clippy → test → docs の翻訳チェック
#   bash scripts/check.sh --quick    # origin/main からの変更があったクレートと、それに依存するクレートだけ
#   bash scripts/check.sh --stage lint   # fmt + clippy だけ（CI の lint ジョブ）
#   bash scripts/check.sh --stage test   # cargo test だけ（CI の test ジョブ）
#
# 最初に失敗した段で止まり、最後の 1 行に `CHECK-OK` か `CHECK-FAILED: <段>` を出す。
# docs の Cypher 例のパース検査（docs_cypher_examples）は cargo test に含まれる。
set -uo pipefail

cd "$(dirname "$0")/.."

quick=0
stage=all
while [ $# -gt 0 ]; do
    case "$1" in
        --quick) quick=1 ;;
        --stage)
            stage="${2:-}"
            shift
            ;;
        -h | --help)
            sed -n '2,10p' "$0"
            exit 0
            ;;
        *)
            echo "unknown option: $1" >&2
            exit 2
            ;;
    esac
    shift
done
case "$stage" in all | lint | test) ;; *)
    echo "--stage は all / lint / test のどれか" >&2
    exit 2
    ;;
esac

current=""
fail() {
    echo
    echo "CHECK-FAILED: $current"
    exit 1
}
run() {
    current="$1"
    shift
    echo "== $current: $*"
    "$@" || fail
}

# --quick: 検査するパッケージ（-p の並び）を決める。空なら Rust の検査は不要。
# "ALL" なら workspace 全体。
packages="ALL"
docs_changed=1
if [ "$quick" = 1 ]; then
    base=$(git merge-base HEAD origin/main 2>/dev/null || echo HEAD)
    changed=$( (git diff --name-only "$base"; git diff --name-only; git ls-files --others --exclude-standard) | sort -u)
    packages=$(printf '%s\n' "$changed" | python3 -c '
import json, subprocess, sys
changed = [l for l in sys.stdin.read().split("\n") if l]
meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--no-deps", "--format-version", "1"]))
root = meta["workspace_root"] + "/"
dirs = {}
for p in meta["packages"]:
    d = p["manifest_path"][len(root):].rsplit("/", 1)[0] if "/" in p["manifest_path"][len(root):] else ""
    dirs[p["name"]] = d
rdeps = {n: set() for n in dirs}
for p in meta["packages"]:
    for d in p["dependencies"]:
        if d.get("path") and d["name"] in rdeps:
            rdeps[d["name"]].add(p["name"])
hit = set()
for f in changed:
    # workspace 全体に効くファイル
    if f in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml") or f.startswith((".cargo/", "clippy.toml")):
        print("ALL"); sys.exit()
    owner = None
    for name, d in dirs.items():
        if d and f.startswith(d + "/"):
            owner = name
    if owner is None and f.startswith(("src/", "examples/", "benches/", "tests/")):
        owner = [n for n, d in dirs.items() if d == ""][0]
    if owner:
        hit.add(owner)
# 依存元をたどる
todo = list(hit)
while todo:
    for r in rdeps[todo.pop()]:
        if r not in hit:
            hit.add(r); todo.append(r)
print(" ".join("-p " + n for n in sorted(hit)))
') || {
        current="--quick の対象決め"
        fail
    }
    if printf '%s\n' "$changed" | grep -q '^docs/'; then docs_changed=1; else docs_changed=0; fi
    echo "== quick: base $base, packages: ${packages:-（なし）}"
fi

if [ "$packages" = "ALL" ]; then
    target=(--workspace)
else
    # shellcheck disable=SC2206
    target=($packages)
fi

if [ "$stage" != test ]; then
    run fmt cargo fmt --all -- --check
    if [ ${#target[@]} -gt 0 ]; then
        run clippy cargo clippy "${target[@]}" --all-targets -- -D warnings
    fi
fi

if [ "$stage" != lint ]; then
    if [ ${#target[@]} -gt 0 ]; then
        run test cargo test "${target[@]}"
    elif [ "$docs_changed" = 1 ]; then
        # docs だけの変更: Cypher 例のパース検査だけ回す
        run docs-cypher cargo test -p maharit-query --test docs_cypher_examples
    fi
fi

if [ "$stage" = all ] && [ "$docs_changed" = 1 ]; then
    run docs-i18n python3 scripts/check_docs_i18n.py
fi

echo
echo "CHECK-OK"
