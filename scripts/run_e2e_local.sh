#!/usr/bin/env bash
# MaharitDB の E2E テスト一式を Docker なしで実行する（CI と同じ手順）。
#
# 使い方:
#   bash scripts/run_e2e_local.sh                     # target/release/maharit を使用
#   bash scripts/run_e2e_local.sh --binary ./target/debug/maharit
#   bash scripts/run_e2e_local.sh --replication-runs 3
#
# 実行内容:
#   1. 単一サーバーを起動し smoke / query_feature / concurrent / constraint / auth を実行
#   2. persistence（自前でサーバーを起動・停止する）
#   3. 3 ノードのレプリケーションクラスターで replication_test を N 回、failover_test を 1 回
#
# いずれかが失敗すると非 0 で終了する。ログは $E2E_LOG_DIR（既定: /tmp/maharit-e2e）に残す。

set -uo pipefail

BINARY="./target/release/maharit"
REPLICATION_RUNS=1
while [[ $# -gt 0 ]]; do
    case "$1" in
        --binary) BINARY="$2"; shift 2 ;;
        --replication-runs) REPLICATION_RUNS="$2"; shift 2 ;;
        *) echo "不明なオプション: $1"; exit 2 ;;
    esac
done

if [[ ! -x "$BINARY" ]]; then
    echo "エラー: バイナリが見つかりません: $BINARY（cargo build --release -p maharit-server）"
    exit 2
fi

LOG_DIR="${E2E_LOG_DIR:-/tmp/maharit-e2e}"
rm -rf "$LOG_DIR"
mkdir -p "$LOG_DIR"
export MAHARIT_E2E_LOCAL=1

FAILED=()
SERVER_PID=""

cleanup() {
    [[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null
    bash scripts/stop_replication_local.sh > /dev/null 2>&1 || true
}
trap cleanup EXIT

wait_port() {
    for _ in $(seq 1 60); do
        python3 -c "import socket; socket.create_connection(('127.0.0.1', $1), 1)" 2>/dev/null && return 0
        sleep 0.5
    done
    return 1
}

run() {
    local name="$1"; shift
    echo "▶ $name"
    if "$@" > "$LOG_DIR/$name.log" 2>&1; then
        echo "  ✓ $(tail -1 "$LOG_DIR/$name.log" | sed 's/\x1b\[[0-9;]*m//g')"
    else
        echo "  ✗ 失敗（$LOG_DIR/$name.log）"
        grep -E '✗|Error|error' "$LOG_DIR/$name.log" | sed 's/\x1b\[[0-9;]*m//g' | head -10 | sed 's/^/    /'
        FAILED+=("$name")
    fi
}

# ── 1. 単一サーバー ────────────────────────────────────────────────────────────
DATA_DIR="$(mktemp -d)"
"$BINARY" server --host 127.0.0.1 --port 7687 --data "$DATA_DIR/e2e.db" > "$LOG_DIR/server.log" 2>&1 &
SERVER_PID=$!
if ! wait_port 7687; then
    echo "エラー: サーバーが起動しません"; cat "$LOG_DIR/server.log"; exit 1
fi
for t in smoke_test query_feature_test concurrent_test constraint_test auth_test; do
    run "$t" python3 "scripts/$t.py"
done
kill "$SERVER_PID" 2>/dev/null; wait "$SERVER_PID" 2>/dev/null; SERVER_PID=""

# ── 2. 永続化 ──────────────────────────────────────────────────────────────────
run persistence_test python3 scripts/persistence_test.py --binary "$BINARY"

# ── 3. レプリケーション ────────────────────────────────────────────────────────
for i in $(seq 1 "$REPLICATION_RUNS"); do
    if ! bash scripts/start_replication_local.sh --binary "$BINARY" > "$LOG_DIR/cluster_$i.log" 2>&1; then
        echo "  ✗ クラスター起動失敗（$LOG_DIR/cluster_$i.log）"
        FAILED+=("cluster_start_$i")
        continue
    fi
    run "replication_test_$i" python3 scripts/replication_test.py
    if [[ "$i" == "$REPLICATION_RUNS" ]]; then
        run failover_test python3 scripts/failover_test.py --no-docker
    fi
    for n in leader follower1 follower2; do
        cp "/tmp/maharit_$n.log" "$LOG_DIR/${n}_$i.log" 2>/dev/null || true
    done
    bash scripts/stop_replication_local.sh > /dev/null 2>&1
done

echo ""
if [[ ${#FAILED[@]} -eq 0 ]]; then
    echo "E2E: 全スイート通過"
    exit 0
fi
echo "E2E: 失敗 ${#FAILED[@]} 件: ${FAILED[*]}"
exit 1
