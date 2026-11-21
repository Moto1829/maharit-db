#!/usr/bin/env bash
# MaharitDB ローカルレプリケーションクラスターを停止する
#
# 使い方:
#   bash scripts/stop_replication_local.sh

PIDFILE="/tmp/maharit_repl_local.pids"

if [[ ! -f "$PIDFILE" ]]; then
    echo "起動中のローカルクラスターが見つかりません (${PIDFILE} なし)"
    exit 0
fi

pids=()
while IFS= read -r pid; do
    if kill "$pid" 2>/dev/null; then
        echo "プロセス $pid を停止しました"
        pids+=("$pid")
    else
        echo "プロセス $pid はすでに終了しています"
    fi
done < "$PIDFILE"

# 終了（停止時のスナップショット保存とポートの解放）を待つ。次の起動がポートを取れずに
# 失敗したり、まだ動いている前のリーダーに繋がったりしないように（bug/202）。
for pid in "${pids[@]+"${pids[@]}"}"; do
    for _ in $(seq 1 60); do
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.25
    done
    if kill -0 "$pid" 2>/dev/null; then
        echo "プロセス $pid が 15 秒で終わらないので強制終了します"
        kill -9 "$pid" 2>/dev/null || true
    fi
done

rm -f "$PIDFILE"
echo "ローカルクラスターを停止しました"
