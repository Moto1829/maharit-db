#!/usr/bin/env python3
"""
MaharitDB 永続化ラウンドトリップテスト

サーバーを起動してデータを書き込み、停止後に再起動してデータが
正しく復元されることを検証する。

使い方:
  python3 scripts/persistence_test.py
  python3 scripts/persistence_test.py --binary ./target/release/maharit
  python3 scripts/persistence_test.py --port 7699 --data /tmp/my_test.db
"""

import argparse
import json
import os
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time

# ── ANSI カラー ───────────────────────────────────────────────────────────────

GREEN  = "\033[92m"
RED    = "\033[91m"
YELLOW = "\033[93m"
CYAN   = "\033[96m"
BOLD   = "\033[1m"
RESET  = "\033[0m"

# ── プロトコル実装（4バイト長プレフィックス + JSON） ─────────────────────────

class MaharitClient:
    def __init__(self, host: str, port: int, timeout: float = 10.0):
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

    def ping(self) -> bool:
        try:
            resp = self.send({"type": "ping"})
            return resp.get("type") == "pong"
        except Exception:
            return False

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
        try:
            self.sock.close()
        except Exception:
            pass


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


def find_binary() -> str:
    """バイナリを探す: release > debug の順"""
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    candidates = [
        os.path.join(root, "target", "release", "maharit"),
        os.path.join(root, "target", "debug", "maharit"),
    ]
    for path in candidates:
        if os.path.isfile(path):
            return path
    raise FileNotFoundError(
        "maharit バイナリが見つかりません。先に `cargo build` を実行してください。"
    )


def start_server(binary: str, port: int, data_path: str) -> subprocess.Popen:
    """サーバープロセスを起動して返す。"""
    proc = subprocess.Popen(
        [binary, "server", "--port", str(port), "--data", data_path],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return proc


def wait_for_server(host: str, port: int, timeout: float = 10.0) -> bool:
    """サーバーが接続を受け付けるまで待機する。"""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            c = MaharitClient(host, port, timeout=1.0)
            if c.ping():
                c.close()
                return True
            c.close()
        except Exception:
            pass
        time.sleep(0.1)
    return False


def stop_server(proc: subprocess.Popen, sig: signal.Signals, wait_secs: float = 5.0) -> bool:
    """シグナルを送りプロセス終了を待つ。タイムアウト内に終了すれば True。"""
    try:
        proc.send_signal(sig)
    except ProcessLookupError:
        return True  # already gone

    try:
        proc.wait(timeout=wait_secs)
        return True
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
        return False


# ── テストスイート ────────────────────────────────────────────────────────────

def test_sigterm_roundtrip(binary: str, port: int, data_path: str):
    """正常終了（SIGTERM）後の再起動でデータが復元されること。"""
    section("SIGTERM ラウンドトリップ")

    # ── フェーズ1: 書き込み ────────────────────────────────────────────────
    proc = start_server(binary, port, data_path)
    ready = wait_for_server("127.0.0.1", port)
    check("サーバー起動", ready)
    if not ready:
        proc.kill(); proc.wait()
        return

    client = MaharitClient("127.0.0.1", port)

    # ノード・エッジ・プロパティを書き込む
    client.query("CREATE (n:Person {name: 'Alice', age: 30})")
    client.query("CREATE (n:Person {name: 'Bob', age: 25})")
    client.query("CREATE (n:City {name: 'Tokyo'})")
    client.query(
        "MATCH (a:Person {name: 'Alice'}), (c:City {name: 'Tokyo'}) "
        "CREATE (a)-[:LIVES_IN]->(c)"
    )

    # 100件の中規模データ
    for i in range(100):
        client.query(f"CREATE (n:Item {{idx: {i}, label: 'item-{i}'}})")

    # 書き込み確認
    resp = client.query("MATCH (n:Person) RETURN n.name")
    before_count = len(resp.get("rows", []))
    check("書き込み前: Person ノード2件", before_count == 2, f"count={before_count}")

    resp = client.query("MATCH (n:Item) RETURN n.idx")
    item_count = len(resp.get("rows", []))
    check("書き込み前: Item ノード100件", item_count == 100, f"count={item_count}")

    client.close()

    # ── SIGTERM で正常終了 ────────────────────────────────────────────────
    clean_exit = stop_server(proc, signal.SIGTERM)
    check("SIGTERM で正常終了", clean_exit)

    check("データファイルが生成された", os.path.isfile(data_path))

    # ── フェーズ2: 再起動 + 読み取り ──────────────────────────────────────
    proc2 = start_server(binary, port, data_path)
    ready2 = wait_for_server("127.0.0.1", port)
    check("再起動後サーバー起動", ready2)
    if not ready2:
        proc2.kill(); proc2.wait()
        return

    client2 = MaharitClient("127.0.0.1", port)

    resp = client2.query("MATCH (n:Person) RETURN n.name")
    rows = resp.get("rows", [])
    check("Person ノード2件が復元", len(rows) == 2, f"count={len(rows)}")

    names = {r.get("n.name", "").strip('"') for r in rows}
    check("Alice が復元", "Alice" in names, f"names={names}")
    check("Bob が復元", "Bob" in names, f"names={names}")

    resp = client2.query("MATCH (n:City {name: 'Tokyo'}) RETURN n.name")
    check("City ノードが復元", len(resp.get("rows", [])) == 1)

    resp = client2.query("MATCH (a:Person)-[:LIVES_IN]->(c:City) RETURN c.name")
    check("エッジ（LIVES_IN）が復元", len(resp.get("rows", [])) >= 1)

    resp = client2.query("MATCH (n:Person {name: 'Alice'}) RETURN n.age")
    rows = resp.get("rows", [])
    check(
        "Alice.age プロパティが復元",
        rows and rows[0].get("n.age") == "30",
        str(rows),
    )

    resp = client2.query("MATCH (n:Item) RETURN n.idx")
    item_count2 = len(resp.get("rows", []))
    check("Item ノード100件が復元", item_count2 == 100, f"count={item_count2}")

    client2.close()
    stop_server(proc2, signal.SIGTERM)


def test_sigkill_roundtrip(binary: str, port: int, data_path: str):
    """強制終了（SIGKILL）後の再起動でサーバーがクラッシュしないこと。"""
    section("SIGKILL 後の再起動")

    # SIGKILL 前に一度 SIGTERM で保存済みデータを用意しておく
    proc = start_server(binary, port, data_path)
    ready = wait_for_server("127.0.0.1", port)
    check("サーバー起動", ready)
    if not ready:
        proc.kill(); proc.wait()
        return

    client = MaharitClient("127.0.0.1", port)
    client.query("CREATE (n:Survivor {name: 'persistent'})")
    client.close()

    # 一旦 SIGTERM で保存
    stop_server(proc, signal.SIGTERM)

    # 再起動してから即 SIGKILL
    proc2 = start_server(binary, port, data_path)
    ready2 = wait_for_server("127.0.0.1", port)
    check("SIGKILL 前サーバー起動", ready2)
    if not ready2:
        proc2.kill(); proc2.wait()
        return

    # 少し書き込んでから SIGKILL（この書き込みは消えてよい）
    client2 = MaharitClient("127.0.0.1", port)
    client2.query("CREATE (n:Volatile {name: 'may_be_lost'})")
    client2.close()

    stop_server(proc2, signal.SIGKILL, wait_secs=3.0)

    # SIGKILL 後の再起動でクラッシュしないことを確認
    proc3 = start_server(binary, port, data_path)
    ready3 = wait_for_server("127.0.0.1", port)
    check("SIGKILL 後の再起動でクラッシュしない", ready3)

    if ready3:
        client3 = MaharitClient("127.0.0.1", port)
        resp = client3.query("MATCH (n:Survivor) RETURN n.name")
        check(
            "SIGTERM 保存データは SIGKILL 後も復元",
            len(resp.get("rows", [])) >= 1,
            str(resp),
        )
        client3.close()
        stop_server(proc3, signal.SIGTERM)
    else:
        proc3.kill(); proc3.wait()


# ── エントリポイント ──────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description="MaharitDB 永続化ラウンドトリップテスト")
    parser.add_argument("--binary", default=None, help="maharit バイナリのパス")
    parser.add_argument("--port", type=int, default=7699, help="テスト用ポート (default: 7699)")
    parser.add_argument("--data", default=None, help="テスト用 DB ファイルパス（省略時は一時ファイル）")
    args = parser.parse_args()

    binary = args.binary or find_binary()
    print(f"バイナリ: {binary}")

    # データファイルは一時ディレクトリに作成（テスト後に削除）
    use_tmpdir = args.data is None
    if use_tmpdir:
        tmpdir = tempfile.mkdtemp(prefix="maharit_persist_test_")
        data_path = os.path.join(tmpdir, "test.db")
    else:
        tmpdir = None
        data_path = args.data

    print(f"データパス: {data_path}")
    print(f"ポート: {args.port}\n")

    try:
        test_sigterm_roundtrip(binary, args.port, data_path)

        # SIGKILL テスト用にデータファイルをリセット
        if os.path.isfile(data_path):
            os.remove(data_path)

        test_sigkill_roundtrip(binary, args.port, data_path)
    finally:
        if use_tmpdir and tmpdir and os.path.isdir(tmpdir):
            shutil.rmtree(tmpdir, ignore_errors=True)

    # ── 結果サマリー ──────────────────────────────────────────────────────
    total = passed + failed
    print(f"\n{BOLD}{'─' * 50}{RESET}")
    if failed == 0:
        print(f"{GREEN}{BOLD}✓ 全 {total} 件通過{RESET}")
    else:
        print(f"{RED}{BOLD}✗ {failed} 件失敗 / {total} 件中{RESET}")

    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
