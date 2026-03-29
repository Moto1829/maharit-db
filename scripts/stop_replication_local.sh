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

while IFS= read -r pid; do
    if kill "$pid" 2>/dev/null; then
        echo "プロセス $pid を停止しました"
    else
        echo "プロセス $pid はすでに終了しています"
    fi
done < "$PIDFILE"

rm -f "$PIDFILE"
echo "ローカルクラスターを停止しました"
