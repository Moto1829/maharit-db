#!/usr/bin/env bash
# MaharitDB レプリケーションクラスターをローカルプロセスで起動する
#
# 使い方:
#   bash scripts/start_replication_local.sh
#   bash scripts/start_replication_local.sh --binary ./target/debug/maharit
#
# 起動構成:
#   リーダー   : localhost:7687  (レプリケーション: 127.0.0.1:7688)
#   フォロワー1 : localhost:7689
#   フォロワー2 : localhost:7690
#
# 停止するには:
#   bash scripts/stop_replication_local.sh

set -euo pipefail

PIDFILE="/tmp/maharit_repl_local.pids"
BINARY=""

# ── 引数解析 ──────────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --binary)
            BINARY="$2"; shift 2 ;;
        *)
            echo "不明なオプション: $1"; exit 1 ;;
    esac
done

# ── バイナリ検出 ──────────────────────────────────────────────────────────────
if [[ -z "$BINARY" ]]; then
    if [[ -f "./target/release/maharit" ]]; then
        BINARY="./target/release/maharit"
    elif [[ -f "./target/debug/maharit" ]]; then
        BINARY="./target/debug/maharit"
    else
        echo "エラー: バイナリが見つかりません。先にビルドしてください:"
        echo "  cargo build -p maharit-server"
        echo "  # または"
        echo "  cargo build --release -p maharit-server"
        exit 1
    fi
fi

echo "バイナリ: $BINARY"

# ── 既存プロセスを停止 ────────────────────────────────────────────────────────
if [[ -f "$PIDFILE" ]]; then
    echo "既存のローカルクラスターを停止します..."
    while IFS= read -r pid; do
        kill "$pid" 2>/dev/null && echo "  PID $pid を停止" || true
    done < "$PIDFILE"
    rm -f "$PIDFILE"
    sleep 1
fi

# ── データファイルをリセット ───────────────────────────────────────────────────
rm -f /tmp/maharit_leader.db /tmp/maharit_follower1.db /tmp/maharit_follower2.db

# ── リーダー起動 ──────────────────────────────────────────────────────────────
echo "リーダー起動中... (port 7687, replication-bind 127.0.0.1:7688)"
"$BINARY" server \
    --host 127.0.0.1 --port 7687 \
    --data /tmp/maharit_leader.db \
    --replication-role leader \
    --replication-bind 127.0.0.1:7688 \
    --node-id leader \
    > /tmp/maharit_leader.log 2>&1 &
echo $! >> "$PIDFILE"

# リーダーが起動するまで待機
sleep 1

# ── フォロワー1 起動 ───────────────────────────────────────────────────────────
echo "フォロワー1 起動中... (port 7689)"
"$BINARY" server \
    --host 127.0.0.1 --port 7689 \
    --data /tmp/maharit_follower1.db \
    --replication-role follower \
    --leader-addr 127.0.0.1:7688 \
    --node-id follower1 \
    > /tmp/maharit_follower1.log 2>&1 &
echo $! >> "$PIDFILE"

# ── フォロワー2 起動 ───────────────────────────────────────────────────────────
echo "フォロワー2 起動中... (port 7690)"
"$BINARY" server \
    --host 127.0.0.1 --port 7690 \
    --data /tmp/maharit_follower2.db \
    --replication-role follower \
    --leader-addr 127.0.0.1:7688 \
    --node-id follower2 \
    > /tmp/maharit_follower2.log 2>&1 &
echo $! >> "$PIDFILE"

# ── 起動完了待機 ──────────────────────────────────────────────────────────────
echo "起動完了を待機中..."
sleep 2

echo ""
echo "クラスター起動完了"
echo "  リーダー   : localhost:7687"
echo "  フォロワー1 : localhost:7689"
echo "  フォロワー2 : localhost:7690"
echo ""
echo "テスト実行:"
echo "  python3 scripts/replication_test.py"
echo "  python3 scripts/failover_test.py --no-docker"
echo ""
echo "ログ確認:"
echo "  tail -f /tmp/maharit_leader.log"
echo "  tail -f /tmp/maharit_follower1.log"
echo ""
echo "停止:"
echo "  bash scripts/stop_replication_local.sh"
