#!/usr/bin/env python3
"""docs の英語版（*.en.md）が日本語の正本に追従しているかを調べる。

英語ファイルの front matter の `source_commit` 以降に、対応する日本語ファイルを
変更したコミットがあれば「古い」と判定する。ただし、同じコミットで英語ファイルも
変更しているものは反映済みとみなす（日英を同時に直したコミットは、自分のハッシュを
`source_commit` に書けないため）。

    python3 scripts/check_docs_i18n.py            # 一覧を表示
    python3 scripts/check_docs_i18n.py --github   # GitHub Actions の注釈とサマリーも出す
    python3 scripts/check_docs_i18n.py --strict   # 古いページがあれば失敗にする

`source_commit` が無い・存在しないコミットを指すときは常に失敗（終了コード 1）。
git の履歴が必要（CI では `fetch-depth: 0`）。mkdocs のフック（`overrides/hooks/i18n_freshness.py`）も
この判定を使う。
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
SKIP_DIRS = {"_i18n"}
FRONT_MATTER = re.compile(r"\A---\n(.*?)\n---\n", re.S)


@dataclass
class Page:
    ja: str  # repo-relative path
    en: str | None
    source_commit: str | None = None
    error: str | None = None
    stale_commits: list[str] = field(default_factory=list)
    added: int = 0
    deleted: int = 0

    @property
    def stale(self) -> bool:
        return bool(self.stale_commits)


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=ROOT, check=True, capture_output=True, text=True
    ).stdout


def has_history() -> bool:
    """浅いクローン（fetch-depth: 1）では判定できない。"""
    try:
        return git("rev-parse", "--is-shallow-repository").strip() == "false"
    except (subprocess.CalledProcessError, FileNotFoundError):
        return False


def source_commit_of(en_path: Path) -> str | None:
    m = FRONT_MATTER.match(en_path.read_text(encoding="utf-8"))
    if not m:
        return None
    for line in m.group(1).splitlines():
        key, _, value = line.partition(":")
        if key.strip() == "source_commit":
            return value.strip().strip("'\"") or None
    return None


def commits_touching(path: str, since: str) -> list[str]:
    out = git("log", "--format=%H", f"{since}..HEAD", "--", path)
    return out.split()


def numstat(commit: str, path: str) -> tuple[int, int]:
    added = deleted = 0
    for line in git("show", "--numstat", "--format=", commit, "--", path).splitlines():
        a, d, _ = line.split("\t", 2)
        if a.isdigit():
            added += int(a)
        if d.isdigit():
            deleted += int(d)
    return added, deleted


def ja_pages() -> list[Path]:
    pages = []
    for p in sorted(DOCS.rglob("*.md")):
        rel = p.relative_to(DOCS)
        if p.name.endswith(".en.md") or rel.parts[0] in SKIP_DIRS:
            continue
        pages.append(p)
    return pages


def check() -> list[Page]:
    pages = []
    for ja in ja_pages():
        en = ja.with_name(ja.name[: -len(".md")] + ".en.md")
        page = Page(
            ja=str(ja.relative_to(ROOT)),
            en=str(en.relative_to(ROOT)) if en.exists() else None,
        )
        pages.append(page)
        if page.en is None:
            continue
        page.source_commit = source_commit_of(en)
        if page.source_commit is None:
            page.error = "front matter に source_commit がありません"
            continue
        try:
            git("rev-parse", "--verify", "--quiet", f"{page.source_commit}^{{commit}}")
        except subprocess.CalledProcessError:
            page.error = f"source_commit {page.source_commit} がコミットとして見つかりません"
            continue
        reflected = set(commits_touching(page.en, page.source_commit))
        for c in commits_touching(page.ja, page.source_commit):
            if c in reflected:
                continue
            page.stale_commits.append(c)
            a, d = numstat(c, page.ja)
            page.added += a
            page.deleted += d
    return pages


def stale_en_paths() -> set[str]:
    """古い英語ページ（docs/ からの相対パス）。履歴が無ければ空。"""
    if not has_history():
        return set()
    return {
        str(Path(p.en).relative_to("docs"))
        for p in check()
        if p.en and p.stale and not p.error
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--github", action="store_true", help="GitHub Actions の注釈・サマリーを出す")
    ap.add_argument("--strict", action="store_true", help="古いページがあれば失敗にする")
    args = ap.parse_args()

    if not has_history():
        print("error: git の履歴がありません（浅いクローンでは判定できません。fetch-depth: 0 で取得してください）")
        return 1

    pages = check()
    translated = [p for p in pages if p.en]
    errors = [p for p in translated if p.error]
    stale = [p for p in translated if p.stale and not p.error]
    missing = [p for p in pages if not p.en]
    rate = 100 * len(translated) / len(pages) if pages else 100.0

    print(f"翻訳率: {len(translated)}/{len(pages)} ページ（{rate:.0f}%）")
    for p in errors:
        print(f"  ERROR {p.en}: {p.error}")
        if args.github:
            print(f"::error file={p.en}::{p.error}")
    for p in stale:
        msg = (
            f"日本語版 {p.ja} が source_commit {p.source_commit[:8]} の後に "
            f"{len(p.stale_commits)} コミット（+{p.added} -{p.deleted} 行）更新されています"
        )
        print(f"  古い {p.en}: {msg}")
        if args.github:
            print(f"::warning file={p.en}::{msg}")
    if not stale and not errors:
        print("  英語版はすべて日本語版に追従しています")

    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if args.github and summary:
        with open(summary, "a", encoding="utf-8") as f:
            f.write("## docs の翻訳状況\n\n")
            f.write(f"翻訳率: **{len(translated)}/{len(pages)} ページ（{rate:.0f}%）**\n\n")
            if errors or stale:
                f.write("| 英語ページ | 状態 |\n|---|---|\n")
                for p in errors:
                    f.write(f"| `{p.en}` | エラー: {p.error} |\n")
                for p in stale:
                    f.write(
                        f"| `{p.en}` | 古い: 日本語が {len(p.stale_commits)} コミット"
                        f"（+{p.added} -{p.deleted} 行）更新 |\n"
                    )
                f.write("\n")
            if missing:
                f.write("<details><summary>英語版の無いページ</summary>\n\n")
                for p in missing:
                    f.write(f"- `{p.ja}`\n")
                f.write("\n</details>\n")

    if errors:
        return 1
    if stale and args.strict:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
