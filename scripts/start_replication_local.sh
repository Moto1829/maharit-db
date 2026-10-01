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

# リーダーが起動するまで待機（ポートが開くまで最大 30 秒）
wait_port() {
    local port=$1
    for _ in $(seq 1 60); do
        if python3 -c "import socket,sys; socket.create_connection(('127.0.0.1', $port), 1)" 2>/dev/null; then
            return 0
        fi
        sleep 0.5
    done
    echo "エラー: port $port が開きません"
    return 1
}
wait_port 7687
wait_port 7688

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
# 固定 sleep だと遅い環境（CI）でフォロワー接続前にテストが始まるため、
# リーダーの stats でフォロワー 2 台の接続を確認するまで待つ（最大 30 秒）。
echo "起動完了を待機中..."
wait_port 7689
wait_port 7690
python3 - <<'PY' || { echo "エラー: フォロワーがリーダーに接続しません"; tail -20 /tmp/maharit_follower1.log /tmp/maharit_follower2.log; exit 1; }
import json, socket, struct, sys, time

def stats():
    s = socket.create_connection(("127.0.0.1", 7687), 2)
    body = json.dumps({"type": "stats"}).encode()
    s.sendall(struct.pack(">I", len(body)) + body)
    n = struct.unpack(">I", s.recv(4))[0]
    data = b""
    while len(data) < n:
        data += s.recv(n - len(data))
    s.close()
    return json.loads(data)

deadline = time.time() + 30
while time.time() < deadline:
    try:
        repl = stats().get("replication") or {}
        if repl.get("follower_count", 0) >= 2:
            sys.exit(0)
    except OSError:
        pass
    time.sleep(0.5)
sys.exit(1)
PY

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
