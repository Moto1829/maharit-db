#!/usr/bin/env bash
# PreToolUse（Bash）: `git commit` を含むコマンドの前に、コミット先のブランチを表示する。
# 共有の作業ディレクトリでは、他のセッションがブランチを切り替えていることがあるため。
# 止めはしない（このリポジトリは main へ直接コミットする運用）。
input=$(cat)
printf '%s' "$input" | python3 -c '
import json, shlex, subprocess, sys
data = json.load(sys.stdin)
cmd = data.get("tool_input", {}).get("command", "")
if "git commit" not in cmd:
    sys.exit(0)
cwd = data.get("cwd") or "."
# `cd <dir> && …` や `git -C <dir>` で別のディレクトリに入るなら、そこのブランチを見る
try:
    words = shlex.split(cmd)
except ValueError:
    words = cmd.split()
for i, w in enumerate(words[:-1]):
    if w == "cd" or (w == "-C" and i > 0 and words[i - 1] == "git"):
        cwd = words[i + 1]
        break
try:
    branch = subprocess.run(["git", "-C", cwd, "branch", "--show-current"],
                            capture_output=True, text=True, timeout=5).stdout.strip()
except Exception:
    sys.exit(0)
if branch:
    print(json.dumps({"systemMessage": f"git commit: ブランチ {branch}（{cwd}）"}, ensure_ascii=False))
'
exit 0
