#!/usr/bin/env bash
# PostToolUse（Edit / Write / MultiEdit）: 編集した .rs ファイルに rustfmt をかける。
# 整形に失敗しても作業は止めない（構文エラーの途中状態でも編集を続けられるように）。
input=$(cat)
file=$(printf '%s' "$input" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("tool_input",{}).get("file_path",""))' 2>/dev/null)
case "$file" in
    *.rs)
        [ -f "$file" ] && rustfmt --edition 2024 --quiet "$file" >/dev/null 2>&1
        ;;
esac
exit 0
