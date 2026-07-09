#!/usr/bin/env python3
"""
MaharitDB レプリケーション動作確認スクリプト

docker-compose.replication.yml で起動したコンテナ群に対して
リーダー → フォロワーへのデータ伝播を検証する。

使い方:
  python3 scripts/replication_test.py
  python3 scripts/replication_test.py --leader-port 7687 --follower-ports 7689,7690
  python3 scripts/replication_test.py --wait 2.0
"""

import argparse
import os
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
    """replication テスト用に addr 属性を保持する。"""

    def __init__(self, host: str, port: int, timeout: float = 10.0):
        super().__init__(host, port, timeout)
        self.addr = f"{host}:{port}"


def connect(host: str, port: int, label: str) -> MaharitClient | None:
    try:
        c = MaharitClient(host, port)
        print(f"  {GREEN}接続成功{RESET} {label} ({host}:{port})")
        return c
    except Exception as e:
        print(f"  {RED}接続失敗{RESET} {label} ({host}:{port}) — {e}")
        return None


# ── テストスイート ────────────────────────────────────────────────────────────

def test_connectivity(leader, followers: list):
    section("接続確認")

    resp = leader.send({"type": "ping"})
    check("リーダー: pong が返る", resp.get("type") == "pong", str(resp))

    for i, f in enumerate(followers, 1):
        if f:
            resp = f.send({"type": "ping"})
            check(f"フォロワー{i}: pong が返る", resp.get("type") == "pong", str(resp))
        else:
            check(f"フォロワー{i}: 接続済み", False, "接続できていない")


def test_leader_stats(leader):
    section("リーダー統計情報")
    resp = leader.send({"type": "stats"})
    check("stats が返る", resp.get("type") == "stats", str(resp))
    check("nodes フィールドがある", "nodes" in resp)
    check("edges フィールドがある", "edges" in resp)


def test_write_to_leader(leader) -> list[str]:
    """リーダーに書き込み、作成したノード名のリストを返す"""
    section("リーダーへの書き込み")

    names = ["Repl_Alice", "Repl_Bob", "Repl_Carol"]
    for name in names:
        resp = leader.query(
            f"CREATE (n:ReplTest {{name: '{name}', ts: {int(time.time())}}}) RETURN n"
        )
        check(f"CREATE {name}", resp.get("type") == "result", str(resp))

    resp = leader.query(
        "MATCH (a:ReplTest {name: 'Repl_Alice'}), (b:ReplTest {name: 'Repl_Bob'}) "
        "CREATE (a)-[:REPL_KNOWS]->(b) RETURN a, b"
    )
    check("エッジ REPL_KNOWS 作成", resp.get("type") == "result", str(resp))

    return names


def test_replication_propagation(leader, followers: list, names: list, wait_sec: float):
    section(f"レプリケーション伝播確認（{wait_sec}秒待機後）")

    print(f"  {YELLOW}⏳ {wait_sec}秒待機中...{RESET}")
    time.sleep(wait_sec)

    # リーダーで確認
    resp = leader.query("MATCH (n:ReplTest) RETURN n.name")
    leader_count = len(resp.get("rows", []))
    check(f"リーダー: {len(names)} 件ある", leader_count == len(names),
          f"count={leader_count}")

    # 各フォロワーで確認
    for i, follower in enumerate(followers, 1):
        if not follower:
            check(f"フォロワー{i}: データ確認", False, "接続なし")
            continue

        resp = follower.query("MATCH (n:ReplTest) RETURN n.name")
        follower_count = len(resp.get("rows", []))
        check(f"フォロワー{i}: {len(names)} 件に伝播", follower_count == len(names),
              f"count={follower_count}")

        # 特定ノードの確認
        resp = follower.query("MATCH (n:ReplTest {name: 'Repl_Alice'}) RETURN n.name")
        rows = resp.get("rows", [])
        check(f"フォロワー{i}: Repl_Alice が存在する", len(rows) == 1, str(rows))

        # エッジの確認
        resp = follower.query(
            "MATCH (a:ReplTest)-[:REPL_KNOWS]->(b:ReplTest) RETURN a.name, b.name"
        )
        edge_rows = resp.get("rows", [])
        check(f"フォロワー{i}: REPL_KNOWS エッジが伝播", len(edge_rows) >= 1,
              f"rows={edge_rows}")


def test_follower_read_consistency(followers: list, names: list):
    section("フォロワー読み取り一貫性")

    for i, follower in enumerate(followers, 1):
        if not follower:
            continue

        resp = follower.query("MATCH (n:ReplTest) RETURN n.name ORDER BY n.name")
        rows = resp.get("rows", [])
        # サーバーは文字列を "\"Alice\"" 形式で返す（REPL 表示用クォート）ので strip する
        actual_names = sorted(r.get("n.name", "").strip('"') for r in rows)
        expected_names = sorted(names)

        check(f"フォロワー{i}: 全ノード名が一致",
              actual_names == expected_names,
              f"expected={expected_names}, actual={actual_names}")


def test_write_propagation_sequential(leader, followers: list, wait_sec: float):
    section("追加書き込みの伝播確認")

    resp = leader.query(
        "CREATE (n:ReplTest {name: 'Repl_Dave', ts: " + str(int(time.time())) + "}) RETURN n"
    )
    check("リーダー: Repl_Dave を追加作成", resp.get("type") == "result", str(resp))

    print(f"  {YELLOW}⏳ {wait_sec}秒待機中...{RESET}")
    time.sleep(wait_sec)

    for i, follower in enumerate(followers, 1):
        if not follower:
            continue
        resp = follower.query("MATCH (n:ReplTest {name: 'Repl_Dave'}) RETURN n.name")
        rows = resp.get("rows", [])
        check(f"フォロワー{i}: Repl_Dave が伝播", len(rows) == 1, str(rows))


def test_cleanup(leader):
    section("クリーンアップ（リーダーから削除）")

    resp = leader.query("MATCH (n:ReplTest) DETACH DELETE n")
    check("全 ReplTest ノードを削除", resp.get("type") == "result", str(resp))

    resp = leader.query("MATCH (n:ReplTest) RETURN n")
    check("削除後 0 件", len(resp.get("rows", [])) == 0,
          f"残り={len(resp.get('rows', []))}")


def test_cleanup_propagation(followers: list, wait_sec: float):
    section(f"削除の伝播確認（{wait_sec}秒待機後）")
    print(f"  {YELLOW}⏳ {wait_sec}秒待機中...{RESET}")
    time.sleep(wait_sec)

    for i, follower in enumerate(followers, 1):
        if not follower:
            continue
        resp = follower.query("MATCH (n:ReplTest) RETURN n")
        check(f"フォロワー{i}: 削除が伝播し 0 件",
              len(resp.get("rows", [])) == 0,
              f"残り={len(resp.get('rows', []))}")


# ── エントリポイント ──────────────────────────────────────────────────────────

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
    parser = argparse.ArgumentParser(description="MaharitDB レプリケーション動作確認")
    parser.add_argument("--host", default="localhost", help="接続ホスト (default: localhost)")
    parser.add_argument("--leader-port", type=int, default=7687, help="リーダーポート (default: 7687)")
    parser.add_argument("--follower-ports", default="7689,7690",
                        help="フォロワーポートのカンマ区切りリスト (default: 7689,7690)")
    parser.add_argument("--wait", type=float, default=1.0,
                        help="レプリケーション伝播を待つ秒数 (default: 1.0)")
    args = parser.parse_args()
    check_docker_compose()

    follower_ports = [int(p.strip()) for p in args.follower_ports.split(",")]

    print(f"\n{BOLD}MaharitDB レプリケーション動作確認{RESET}")
    print(f"リーダー  : {args.host}:{args.leader_port}")
    for i, p in enumerate(follower_ports, 1):
        print(f"フォロワー{i}: {args.host}:{p}")
    print(f"待機時間  : {args.wait}秒")
    print("─" * 50)

    section("接続")
    leader = connect(args.host, args.leader_port, "リーダー")
    if not leader:
        print(f"\n{RED}リーダーに接続できません。コンテナが起動しているか確認してください。{RESET}")
        print("  docker compose -f docker-compose.replication.yml up -d")
        sys.exit(1)

    followers = [connect(args.host, p, f"フォロワー{i}") for i, p in enumerate(follower_ports, 1)]

    try:
        test_connectivity(leader, followers)
        test_leader_stats(leader)
        names = test_write_to_leader(leader)
        test_replication_propagation(leader, followers, names, args.wait)
        test_follower_read_consistency(followers, names)
        test_write_propagation_sequential(leader, followers, args.wait)
        test_cleanup(leader)
        test_cleanup_propagation(followers, args.wait)
    finally:
        leader.close()
        for f in followers:
            if f:
                f.close()

    sys.exit(summarize())


if __name__ == "__main__":
    main()
