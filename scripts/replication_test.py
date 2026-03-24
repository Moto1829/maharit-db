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
import json
import socket
import struct
import sys
import time

# ── ANSI カラー ──────────────────────────────────────────────────────────────
GREEN  = "\033[92m"
RED    = "\033[91m"
YELLOW = "\033[93m"
CYAN   = "\033[96m"
BOLD   = "\033[1m"
RESET  = "\033[0m"

# ── プロトコル実装（4バイト長プレフィックス + JSON） ─────────────────────────

class MaharitClient:
    def __init__(self, host: str, port: int, timeout: float = 10.0):
        self.addr = f"{host}:{port}"
        self.sock = socket.create_connection((host, port), timeout=timeout)

    def send(self, request: dict) -> dict:
        data = json.dumps(request).encode()
        self.sock.sendall(struct.pack(">I", len(data)) + data)
        raw_len = self._recv_exactly(4)
        length = struct.unpack(">I", raw_len)[0]
        payload = self._recv_exactly(length)
        return json.loads(payload)

    def query(self, cypher: str) -> dict:
        return self.send({"type": "query", "query": cypher})

    def _recv_exactly(self, n: int) -> bytes:
        buf = b""
        while len(buf) < n:
            chunk = self.sock.recv(n - len(buf))
            if not chunk:
                raise ConnectionError("Connection closed by server")
            buf += chunk
        return buf

    def close(self):
        try:
            self.send({"type": "disconnect"})
        except Exception:
            pass
        self.sock.close()


# ── テストヘルパー ────────────────────────────────────────────────────────────

passed = 0
failed = 0


def check(name: str, condition: bool, detail: str = ""):
    global passed, failed
    if condition:
        passed += 1
        print(f"  {GREEN}✓{RESET} {name}")
    else:
        failed += 1
        detail_str = f" — {detail}" if detail else ""
        print(f"  {RED}✗{RESET} {name}{detail_str}")


def section(title: str):
    print(f"\n{CYAN}{BOLD}▶ {title}{RESET}")


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
        actual_names = sorted(r.get("n.name", "") for r in rows)
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

def main():
    parser = argparse.ArgumentParser(description="MaharitDB レプリケーション動作確認")
    parser.add_argument("--host", default="localhost", help="接続ホスト (default: localhost)")
    parser.add_argument("--leader-port", type=int, default=7687, help="リーダーポート (default: 7687)")
    parser.add_argument("--follower-ports", default="7689,7690",
                        help="フォロワーポートのカンマ区切りリスト (default: 7689,7690)")
    parser.add_argument("--wait", type=float, default=1.0,
                        help="レプリケーション伝播を待つ秒数 (default: 1.0)")
    args = parser.parse_args()

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

    print(f"\n{'─' * 50}")
    print(f"{BOLD}結果: {GREEN}{passed} passed{RESET}", end="")
    if failed:
        print(f", {RED}{failed} failed{RESET}")
    else:
        print()
    print()

    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
