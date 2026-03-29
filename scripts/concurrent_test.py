#!/usr/bin/env python3
"""
MaharitDB 同時接続テスト (E2E)

複数クライアントが同時に接続・書き込みを行う状況でのデータ整合性と
サーバーの安定性を検証する。

使い方:
  python3 scripts/concurrent_test.py
  python3 scripts/concurrent_test.py --host localhost --port 7687
"""

import argparse
import json
import socket
import struct
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor, as_completed

# ── ANSI カラー ──────────────────────────────────────────────────────────────
GREEN = "\033[92m"
RED   = "\033[91m"
CYAN  = "\033[96m"
YELLOW = "\033[93m"
BOLD  = "\033[1m"
RESET = "\033[0m"

HOST = "localhost"
PORT = 7687

# ── プロトコル実装（4バイト長プレフィックス + JSON） ─────────────────────────

class MaharitClient:
    """スレッドセーフではないので、スレッドごとに生成すること。"""

    def __init__(self, host: str, port: int, timeout: float = 10.0):
        self.sock = socket.create_connection((host, port), timeout=timeout)

    def send(self, request: dict) -> dict:
        data = json.dumps(request).encode()
        self.sock.sendall(struct.pack(">I", len(data)) + data)
        return self._recv_one()

    def query(self, q: str, tx_id: int | None = None) -> dict:
        req: dict = {"type": "query", "query": q}
        if tx_id is not None:
            req["txId"] = tx_id
        return self.send(req)

    def begin(self) -> int:
        resp = self.send({"type": "begin", "readOnly": False})
        if resp.get("type") != "transactionBegun":
            raise RuntimeError(f"BEGIN failed: {resp}")
        return resp["txId"]

    def commit(self, tx_id: int) -> None:
        resp = self.send({"type": "commit", "txId": tx_id})
        if resp.get("type") != "committed":
            raise RuntimeError(f"COMMIT failed: {resp}")

    def rollback(self, tx_id: int) -> None:
        resp = self.send({"type": "rollback", "txId": tx_id})
        if resp.get("type") != "rolledBack":
            raise RuntimeError(f"ROLLBACK failed: {resp}")

    def _recv_one(self) -> dict:
        raw_len = self._recv_exactly(4)
        length = struct.unpack(">I", raw_len)[0]
        payload = self._recv_exactly(length)
        return json.loads(payload)

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


def new_client() -> MaharitClient:
    return MaharitClient(HOST, PORT)


# ── テストヘルパー ────────────────────────────────────────────────────────────

passed = 0
failed = 0
_lock = threading.Lock()


def check(name: str, condition: bool, detail: str = ""):
    global passed, failed
    with _lock:
        if condition:
            passed += 1
            print(f"  {GREEN}✓{RESET} {name}")
        else:
            failed += 1
            detail_str = f" — {detail}" if detail else ""
            print(f"  {RED}✗{RESET} {name}{detail_str}")


def section(title: str):
    print(f"\n{CYAN}{BOLD}▶ {title}{RESET}")


# ── テストスイート ────────────────────────────────────────────────────────────

def test_concurrent_writes():
    """10スレッドが同時に異なるノードを作成し、全件が保存されることを確認。"""
    section("同時書き込み (10スレッド)")

    errors: list[str] = []

    def write_node(i: int):
        try:
            c = new_client()
            resp = c.query(f"CREATE (n:ConcWrite {{id: {i}}}) RETURN n")
            if resp.get("type") != "result":
                errors.append(f"thread {i}: {resp}")
            c.close()
        except Exception as e:
            errors.append(f"thread {i}: {e}")

    threads = [threading.Thread(target=write_node, args=(i,)) for i in range(10)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    check("全スレッドがエラーなく完了", len(errors) == 0,
          "; ".join(errors) if errors else "")

    # 件数確認
    c = new_client()
    resp = c.query("MATCH (n:ConcWrite) RETURN n.id")
    c.close()
    count = len(resp.get("rows", []))
    check("10件のノードが作成されている", count == 10, f"実際: {count}件")


def test_concurrent_read_write_mix():
    """5スレッドが書き込み、5スレッドが読み取りを同時実行してもクラッシュしない。"""
    section("読み書き混在 (5ライター + 5リーダー)")

    # セットアップ: 読み取り用ノード
    setup = new_client()
    setup.query("CREATE (n:ReadTarget {val: 1})")
    setup.close()

    errors: list[str] = []

    def writer(i: int):
        try:
            c = new_client()
            resp = c.query(f"CREATE (n:RWWrite {{id: {i}}}) RETURN n")
            if resp.get("type") != "result":
                errors.append(f"writer {i}: {resp}")
            c.close()
        except Exception as e:
            errors.append(f"writer {i}: {e}")

    def reader(i: int):
        try:
            c = new_client()
            resp = c.query("MATCH (n:ReadTarget) RETURN n.val")
            if resp.get("type") != "result":
                errors.append(f"reader {i}: {resp}")
            c.close()
        except Exception as e:
            errors.append(f"reader {i}: {e}")

    threads = (
        [threading.Thread(target=writer, args=(i,)) for i in range(5)]
        + [threading.Thread(target=reader, args=(i,)) for i in range(5)]
    )
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    check("全スレッドがエラーなく完了", len(errors) == 0,
          "; ".join(errors) if errors else "")
    check("サーバーがまだ応答する", _ping_ok(), "ping failed")


def _ping_ok() -> bool:
    try:
        c = new_client()
        resp = c.send({"type": "ping"})
        c.close()
        return resp.get("type") == "pong"
    except Exception:
        return False


def test_max_connections_burst():
    """20クライアントが同時に接続を試みてもサーバーがクラッシュしない。"""
    section("大量同時接続 (20クライアント)")

    results: list[bool] = []
    errors: list[str] = []

    def connect_and_ping(i: int):
        try:
            c = MaharitClient(HOST, PORT, timeout=10.0)
            resp = c.send({"type": "ping"})
            ok = resp.get("type") == "pong"
            results.append(ok)
            c.close()
        except Exception as e:
            errors.append(f"client {i}: {e}")

    with ThreadPoolExecutor(max_workers=20) as ex:
        futs = [ex.submit(connect_and_ping, i) for i in range(20)]
        for f in as_completed(futs):
            f.result()  # 例外を再送出させる

    connected = len(results)
    succeeded = sum(results)
    check(f"{connected}件が接続に成功", connected > 0, f"succeeded={succeeded}")
    check("サーバーがまだ応答する", _ping_ok(), "final ping failed")
    if errors:
        print(f"  {YELLOW}⚠ 一部クライアントが接続できませんでした (制限超過の可能性): "
              f"{len(errors)}件{RESET}")


def test_transaction_isolation():
    """AがCOMMIT・BがROLLBACKしたとき、Aのノードのみ残ることを確認。"""
    section("トランザクション分離")

    client_a = new_client()
    client_b = new_client()

    try:
        tx_a = client_a.begin()
        tx_b = client_b.begin()

        resp_a = client_a.query("CREATE (n:TxIsolate {owner: 'A'}) RETURN n", tx_id=tx_a)
        check("A: トランザクション内でノード作成", resp_a.get("type") == "result", str(resp_a))

        resp_b = client_b.query("CREATE (n:TxIsolate {owner: 'B'}) RETURN n", tx_id=tx_b)
        check("B: トランザクション内でノード作成", resp_b.get("type") == "result", str(resp_b))

        client_a.commit(tx_a)
        check("A: COMMIT 成功", True)

        client_b.rollback(tx_b)
        check("B: ROLLBACK 成功", True)

    except Exception as e:
        check("トランザクション操作", False, str(e))
    finally:
        client_a.close()
        client_b.close()

    # 確認
    checker = new_client()
    resp = checker.query("MATCH (n:TxIsolate) RETURN n.owner")
    checker.close()

    owners = [row.get("n.owner", "") for row in resp.get("rows", [])]
    check('A のノードが存在する ("A"がある)', '"A"' in owners, f"owners={owners}")
    check('B のノードが消えている ("B"がない)', '"B"' not in owners, f"owners={owners}")


def test_concurrent_transactions():
    """複数スレッドが同時にトランザクションを開始・コミットしても整合性を保つ。"""
    section("同時トランザクション (5スレッド)")

    errors: list[str] = []

    def tx_write(i: int):
        try:
            c = new_client()
            tx_id = c.begin()
            c.query(f"CREATE (n:ConcTx {{id: {i}}}) RETURN n", tx_id=tx_id)
            c.commit(tx_id)
            c.close()
        except Exception as e:
            errors.append(f"tx {i}: {e}")

    threads = [threading.Thread(target=tx_write, args=(i,)) for i in range(5)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    check("全トランザクションがエラーなく完了", len(errors) == 0,
          "; ".join(errors) if errors else "")

    c = new_client()
    resp = c.query("MATCH (n:ConcTx) RETURN n.id")
    c.close()
    count = len(resp.get("rows", []))
    check("5件のノードがコミットされている", count == 5, f"実際: {count}件")


def cleanup():
    """テストで作成したノードを削除する。"""
    section("クリーンアップ")
    c = new_client()
    for label in ("ConcWrite", "ReadTarget", "RWWrite", "TxIsolate", "ConcTx"):
        resp = c.query(f"MATCH (n:{label}) DETACH DELETE n")
        ok = resp.get("type") == "result"
        check(f"{label} 削除", ok, str(resp) if not ok else "")
    c.close()


# ── エントリポイント ──────────────────────────────────────────────────────────

def check_docker_compose() -> None:
    """docker-compose.yml の maharit-db-server が起動中か確認する。"""
    try:
        result = subprocess.run(
            ["docker", "ps", "--format", "{{.Names}}"],
            capture_output=True, text=True, timeout=5,
        )
        running = set(result.stdout.splitlines())
    except Exception:
        print(f"{YELLOW}警告: Docker が利用できないため前提環境の確認をスキップします。{RESET}")
        return

    if "maharit-db-server" in running:
        return

    if running & {"maharit-leader", "maharit-follower1", "maharit-follower2"}:
        print(f"\n{RED}エラー: docker-compose.replication.yml の環境が起動中です。{RESET}")
        print("このスクリプトには docker-compose.yml (シングルサーバー) が必要です。\n")
        print("  docker compose -f docker-compose.replication.yml down")
        print("  docker compose up -d maharit-server\n")
        sys.exit(1)

    print(f"\n{RED}エラー: maharit-db-server コンテナが起動していません。{RESET}")
    print("以下のコマンドで起動してください:\n")
    print("  docker compose up -d maharit-server\n")
    sys.exit(1)


def main():
    global HOST, PORT

    parser = argparse.ArgumentParser(description="MaharitDB 同時接続テスト")
    parser.add_argument("--host", default="localhost")
    parser.add_argument("--port", type=int, default=7687)
    args = parser.parse_args()
    check_docker_compose()

    HOST = args.host
    PORT = args.port

    print(f"\n{BOLD}MaharitDB 同時接続テスト (E2E){RESET}")
    print(f"接続先: {HOST}:{PORT}")
    print("─" * 40)

    # 事前接続確認
    if not _ping_ok():
        print(f"{RED}接続失敗: {HOST}:{PORT} に接続できません。{RESET}")
        print("サーバーを起動してから再実行してください:")
        print("  cargo run -p maharit-server")
        sys.exit(1)

    try:
        test_concurrent_writes()
        test_concurrent_read_write_mix()
        test_max_connections_burst()
        test_transaction_isolation()
        test_concurrent_transactions()
    finally:
        cleanup()

    print(f"\n{'─' * 40}")
    print(f"{BOLD}結果: {GREEN}{passed} passed{RESET}", end="")
    if failed:
        print(f", {RED}{failed} failed{RESET}")
    else:
        print()

    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
