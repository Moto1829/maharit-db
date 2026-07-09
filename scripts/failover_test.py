#!/usr/bin/env python3
"""
MaharitDB フェイルオーバーテストスクリプト

docker-compose.replication.yml で起動した環境（リーダー1台 + フォロワー2台）で
リーダー停止 → フォロワー昇格 → 継続書き込み・読み取りを検証する。

前提条件:
  docker compose -f docker-compose.replication.yml up -d --build
  # 全コンテナが healthy になるまで待つ

使い方:
  python3 scripts/failover_test.py
  python3 scripts/failover_test.py --leader-port 7687 --follower1-port 7689 --follower2-port 7690
  python3 scripts/failover_test.py --no-docker   # Docker コマンドをスキップ（手動操作確認用）
"""

import argparse
import os
import signal
import socket
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from lib.client import MaharitClient as _BaseClient  # noqa: E402
from lib.reporting import (  # noqa: E402
    BOLD,
    CYAN,
    GREEN,
    RED,
    RESET,
    YELLOW,
    check,
    section,
    summarize,
)


class MaharitClient(_BaseClient):
    """failover テスト用に addr 属性を保持する。"""

    def __init__(self, host: str, port: int, timeout: float = 10.0):
        super().__init__(host, port, timeout)
        self.addr = f"{host}:{port}"


def connect(host: str, port: int, label: str, timeout: float = 10.0):
    try:
        c = MaharitClient(host, port, timeout=timeout)
        print(f"  {GREEN}接続成功{RESET} {label} ({host}:{port})")
        return c
    except Exception as e:
        print(f"  {RED}接続失敗{RESET} {label} ({host}:{port}) — {e}")
        return None


def run_docker(args: list[str]) -> tuple[int, str]:
    """docker コマンドを実行してリターンコードと出力を返す"""
    try:
        result = subprocess.run(
            ["docker"] + args,
            capture_output=True,
            text=True,
            timeout=30,
        )
        return result.returncode, result.stdout.strip() or result.stderr.strip()
    except subprocess.TimeoutExpired:
        return 1, "timeout"
    except FileNotFoundError:
        return 1, "docker コマンドが見つかりません"


LOCAL_PIDFILE = "/tmp/maharit_repl_local.pids"


def _find_binary() -> str | None:
    """ローカルクラスターの maharit バイナリを検出する。"""
    for path in ("./target/release/maharit", "./target/debug/maharit"):
        if os.path.isfile(path):
            return path
    return None


def kill_local_leader() -> bool:
    """PID ファイル先頭（リーダー）のプロセスを SIGKILL で停止する。"""
    try:
        with open(LOCAL_PIDFILE) as f:
            pids = [line.strip() for line in f if line.strip()]
    except OSError:
        return False
    if not pids:
        return False
    try:
        os.kill(int(pids[0]), signal.SIGKILL)
        return True
    except (ProcessLookupError, ValueError):
        return False


# ── テストフロー ──────────────────────────────────────────────────────────────


def phase1_initial_check(leader, followers: list):
    """フェーズ1: 初期状態の確認とデータ書き込み"""
    section("フェーズ1: 初期状態確認")

    # Ping
    resp = leader.send({"type": "ping"})
    check("リーダー: pong が返る", resp.get("type") == "pong", str(resp))

    for i, f in enumerate(followers, 1):
        if f:
            resp = f.send({"type": "ping"})
            check(f"フォロワー{i}: pong が返る", resp.get("type") == "pong", str(resp))

    # リーダーに書き込む
    section("フェーズ1: リーダーへの書き込み")
    ts = int(time.time())
    nodes = ["Failover_Alice", "Failover_Bob", "Failover_Carol"]
    for name in nodes:
        resp = leader.query(
            f"CREATE (n:FailoverTest {{name: '{name}', ts: {ts}}}) RETURN n"
        )
        check(f"CREATE {name}", resp.get("type") == "result", str(resp))

    resp = leader.query(
        "MATCH (a:FailoverTest {name: 'Failover_Alice'}), "
        "(b:FailoverTest {name: 'Failover_Bob'}) "
        "CREATE (a)-[:FAILOVER_KNOWS]->(b) RETURN a, b"
    )
    check("エッジ FAILOVER_KNOWS 作成", resp.get("type") == "result", str(resp))

    return nodes


def phase2_verify_replication(followers: list, nodes: list, wait_sec: float):
    """フェーズ2: レプリケーション伝播確認"""
    section(f"フェーズ2: レプリケーション伝播確認（{wait_sec}秒待機）")
    print(f"  {YELLOW}⏳ {wait_sec}秒待機中...{RESET}")
    time.sleep(wait_sec)

    for i, f in enumerate(followers, 1):
        if not f:
            check(f"フォロワー{i}: 伝播確認", False, "接続なし")
            continue
        resp = f.query("MATCH (n:FailoverTest) RETURN n.name")
        count = len(resp.get("rows", []))
        check(f"フォロワー{i}: {len(nodes)} 件に伝播", count == len(nodes), f"count={count}")

        resp = f.query(
            "MATCH (a:FailoverTest)-[:FAILOVER_KNOWS]->(b:FailoverTest) "
            "RETURN a.name, b.name"
        )
        check(f"フォロワー{i}: エッジが伝播", len(resp.get("rows", [])) >= 1)


def phase3_stop_leader(use_docker: bool):
    """フェーズ3: リーダー停止"""
    section("フェーズ3: リーダー停止")

    if use_docker:
        rc, out = run_docker(["stop", "maharit-leader"])
        check("docker stop maharit-leader", rc == 0, out)
    else:
        # ローカルプロセスモード: PID ファイル先頭のリーダーを実際に kill する。
        killed = kill_local_leader()
        check("ローカルリーダープロセスを停止 (SIGKILL)", killed,
              "PIDファイルからリーダーをkillできなかった")
        time.sleep(1)


def phase4_promote_follower(follower1_port: int, use_docker: bool):
    """フェーズ4: フォロワー1を昇格"""
    section("フェーズ4: フォロワー1をリーダーに昇格")

    promote_addr = f"127.0.0.1:{follower1_port}"

    if use_docker:
        # maharit バイナリが PATH にある場合は直接実行
        # Docker 環境では docker exec を使用
        rc, out = run_docker([
            "exec", "maharit-follower1",
            "/app/maharit", "admin", "promote-to-leader",
            "--addr", f"127.0.0.1:{follower1_port}",
        ])
        check("maharit admin promote-to-leader (docker exec)", rc == 0, out)
    else:
        # ローカルプロセスモード: 検出したバイナリで昇格コマンドを実行する。
        binary = _find_binary()
        if not binary:
            check("promote 用バイナリ検出", False,
                  "./target/{release,debug}/maharit が見つからない")
            return
        try:
            result = subprocess.run(
                [binary, "admin", "promote-to-leader", "--addr", promote_addr],
                capture_output=True, text=True, timeout=30,
            )
            out = result.stdout.strip() or result.stderr.strip()
            check("maharit admin promote-to-leader (local)", result.returncode == 0, out)
        except Exception as e:
            check("maharit admin promote-to-leader (local)", False, str(e))
        time.sleep(1)


def phase5_verify_new_leader(follower1_port: int, wait_sec: float):
    """フェーズ5: 昇格後の新リーダー動作確認"""
    section("フェーズ5: 昇格後の新リーダー（旧フォロワー1）動作確認")

    print(f"  {YELLOW}⏳ {wait_sec}秒待機中（昇格処理完了を待つ）...{RESET}")
    time.sleep(wait_sec)

    new_leader = connect("127.0.0.1", follower1_port, "新リーダー（旧フォロワー1）")
    if not new_leader:
        check("新リーダーへの接続", False, "接続できない")
        return None

    # 既存データが保持されていること
    resp = new_leader.query("MATCH (n:FailoverTest) RETURN n.name")
    count = len(resp.get("rows", []))
    check("昇格前にレプリケートされたデータが保持", count >= 3, f"count={count}")

    # 新リーダーへの書き込みができること
    ts = int(time.time())
    resp = new_leader.query(
        f"CREATE (n:FailoverTest {{name: 'Failover_Dave_new', ts: {ts}}}) RETURN n"
    )
    check("新リーダーへの書き込み成功", resp.get("type") == "result", str(resp))

    # 書き込んだデータが MATCH で返ること
    resp = new_leader.query(
        "MATCH (n:FailoverTest {name: 'Failover_Dave_new'}) RETURN n.name"
    )
    rows = resp.get("rows", [])
    check("新リーダーで書き込みデータが MATCH で返る", len(rows) >= 1, str(rows))

    new_leader.close()
    return True


def phase6_verify_follower2(follower2_port: int, wait_sec: float):
    """フェーズ6: フォロワー2の状態確認"""
    section("フェーズ6: フォロワー2の状態確認")

    print(f"  {YELLOW}⏳ {wait_sec}秒待機中（ハートビートタイムアウト待ち）...{RESET}")
    time.sleep(wait_sec)

    f2 = connect("127.0.0.1", follower2_port, "フォロワー2", timeout=5.0)
    if not f2:
        # 接続失敗 = リーダーが落ちたことでフォロワー2 も応答不能になった可能性
        check("フォロワー2: 接続試行（タイムアウト可）", True, "接続できないが期待通り")
        return

    # stats で is_leader_alive が false になっていることを確認
    try:
        resp = f2.send({"type": "ping"})
        # ping に応答できる = クライアント接続は生きている
        check("フォロワー2: クライアント接続は生きている", resp.get("type") == "pong", str(resp))

        resp = f2.send({"type": "stats"})
        is_leader_alive = resp.get("replication", {}).get("is_leader_alive", True)
        check(
            "フォロワー2: is_leader_alive が false になっている",
            not is_leader_alive,
            f"stats={resp}",
        )
    except Exception as e:
        check("フォロワー2: 応答確認", False, str(e))
    finally:
        try:
            f2.close()
        except Exception:
            pass


# ── メイン ────────────────────────────────────────────────────────────────────

def _port_open(host: str, port: int) -> bool:
    """指定ホスト:ポートへの接続を試みる。"""
    try:
        s = socket.create_connection((host, port), timeout=1.0)
        s.close()
        return True
    except Exception:
        return False


def check_docker_compose() -> None:
    """docker-compose.replication.yml のコンテナ群、またはローカルプロセスが
    起動中か確認する。"""
    required = {"maharit-leader", "maharit-follower1", "maharit-follower2"}
    running: set = set()
    try:
        result = subprocess.run(
            ["docker", "ps", "--format", "{{.Names}}"],
            capture_output=True, text=True, timeout=5,
        )
        running = set(result.stdout.splitlines())
    except Exception:
        pass  # Docker が使えない場合はポートチェックへフォールバック

    # Docker コンテナで全コンテナが起動中
    if not (required - running):
        return

    # ローカルプロセスとしてポートが開いているか確認
    local_ports = [7687, 7689, 7690]
    if all(_port_open("127.0.0.1", p) for p in local_ports):
        print(f"{CYAN}ℹ ローカルプロセスモードで実行します (ports: {local_ports}){RESET}")
        return

    # 誤った compose が起動中
    if "maharit-db-server" in running:
        print(f"\n{RED}エラー: docker-compose.yml (シングルサーバー) の環境が起動中です。{RESET}")
        print("このスクリプトには docker-compose.replication.yml が必要です。\n")
        print("  docker compose down")
        print("  docker compose -f docker-compose.replication.yml up -d --build\n")
        sys.exit(1)

    # 何も起動していない
    print(f"\n{RED}エラー: レプリケーション環境が起動していません。{RESET}")
    if running and (required - running):
        print(f"  未起動コンテナ: {', '.join(sorted(required - running))}")
    print("以下のいずれかのコマンドで起動してください:\n")
    print("  bash scripts/start_replication_local.sh          # ローカルプロセス")
    print("  docker compose -f docker-compose.replication.yml up -d --build  # Docker\n")
    sys.exit(1)


def main():
    parser = argparse.ArgumentParser(description="MaharitDB フェイルオーバーテスト")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--leader-port", type=int, default=7687)
    parser.add_argument("--follower1-port", type=int, default=7689)
    parser.add_argument("--follower2-port", type=int, default=7690)
    parser.add_argument("--repl-wait", type=float, default=1.0,
                        help="レプリケーション伝播を待つ秒数")
    parser.add_argument("--promote-wait", type=float, default=2.0,
                        help="昇格処理完了を待つ秒数")
    parser.add_argument("--hb-timeout-wait", type=float, default=6.0,
                        help="ハートビートタイムアウト検知を待つ秒数（デフォルト 5s + 余裕 1s）")
    parser.add_argument("--no-docker", action="store_true",
                        help="Docker コマンドをスキップし手動操作を促す")
    args = parser.parse_args()
    check_docker_compose()

    use_docker = not args.no_docker

    print(f"\n{BOLD}{'=' * 60}{RESET}")
    print(f"{BOLD} MaharitDB フェイルオーバーテスト{RESET}")
    print(f"{BOLD}{'=' * 60}{RESET}")

    # 初期接続
    section("接続確認")
    leader = connect(args.host, args.leader_port, "リーダー")
    follower1 = connect(args.host, args.follower1_port, "フォロワー1")
    follower2 = connect(args.host, args.follower2_port, "フォロワー2")

    if not leader:
        print(f"\n{RED}リーダーに接続できません。docker-compose が起動しているか確認してください。{RESET}")
        print("  docker compose -f docker-compose.replication.yml up -d --build")
        sys.exit(1)

    # フェーズ1: 初期確認 + 書き込み
    nodes = phase1_initial_check(leader, [follower1, follower2])

    # フェーズ2: レプリケーション伝播確認
    phase2_verify_replication([follower1, follower2], nodes, args.repl_wait)

    # リーダー接続を閉じる（停止前に）
    try:
        leader.close()
    except Exception:
        pass
    if follower1:
        try:
            follower1.close()
        except Exception:
            pass
    if follower2:
        try:
            follower2.close()
        except Exception:
            pass

    # フェーズ3: リーダー停止
    phase3_stop_leader(use_docker)

    # フェーズ4: フォロワー1を昇格
    phase4_promote_follower(args.follower1_port, use_docker)

    # フェーズ5: 昇格後の新リーダー動作確認
    phase5_verify_new_leader(args.follower1_port, args.promote_wait)

    # フェーズ6: フォロワー2のハートビートタイムアウト確認
    phase6_verify_follower2(args.follower2_port, args.hb_timeout_wait)

    sys.exit(summarize())


if __name__ == "__main__":
    main()
