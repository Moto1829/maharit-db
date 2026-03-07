#!/usr/bin/env python3
"""
MaharitDB Smoke Test
ローカルで起動中の maharit-db に対して各種操作を行い動作確認する。

使い方:
  python3 scripts/smoke_test.py
  python3 scripts/smoke_test.py --host localhost --port 7687
"""

import argparse
import json
import socket
import struct
import sys
import time

# ── ANSI カラー ──────────────────────────────────────────────────────────────
GREEN = "\033[92m"
RED   = "\033[91m"
CYAN  = "\033[96m"
BOLD  = "\033[1m"
RESET = "\033[0m"


# ── プロトコル実装（4バイト長プレフィックス + JSON） ─────────────────────────

class MaharitClient:
    def __init__(self, host: str, port: int, timeout: float = 10.0):
        self.sock = socket.create_connection((host, port), timeout=timeout)

    def send(self, request: dict) -> dict | list[dict]:
        """リクエストを送信し、レスポンスを返す。ストリーミングの場合はリストで返す。"""
        data = json.dumps(request).encode()
        self.sock.sendall(struct.pack(">I", len(data)) + data)

        resp_type = request.get("type")
        if resp_type == "streamQuery":
            return self._recv_stream()
        return self._recv_one()

    def _recv_one(self) -> dict:
        raw_len = self._recv_exactly(4)
        length = struct.unpack(">I", raw_len)[0]
        payload = self._recv_exactly(length)
        return json.loads(payload)

    def _recv_stream(self) -> list[dict]:
        """StreamStart → StreamChunk* → StreamEnd をまとめて受信する。"""
        messages = []
        while True:
            msg = self._recv_one()
            messages.append(msg)
            if msg.get("type") in ("streamEnd", "error"):
                break
        return messages

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


def run_query(client: MaharitClient, query: str) -> dict:
    return client.send({"type": "query", "query": query})


# ── テストスイート ────────────────────────────────────────────────────────────

def test_ping(client: MaharitClient):
    section("Ping")
    resp = client.send({"type": "ping"})
    check("pong が返る", resp.get("type") == "pong", str(resp))


def test_stats(client: MaharitClient):
    section("Stats")
    resp = client.send({"type": "stats"})
    check("stats が返る", resp.get("type") == "stats", str(resp))
    check("nodes フィールドがある", "nodes" in resp)
    check("edges フィールドがある", "edges" in resp)


def test_create_nodes(client: MaharitClient):
    section("CREATE ノード")

    resp = run_query(client, "CREATE (a:Person {name: 'Alice', age: 30}) RETURN a")
    check("Alice 作成", resp.get("type") == "result", str(resp))

    resp = run_query(client, "CREATE (b:Person {name: 'Bob', age: 25}) RETURN b")
    check("Bob 作成", resp.get("type") == "result", str(resp))

    resp = run_query(client, "CREATE (c:Company {name: 'Acme'}) RETURN c")
    check("Acme 作成", resp.get("type") == "result", str(resp))


def test_create_edges(client: MaharitClient):
    section("CREATE エッジ")

    resp = run_query(
        client,
        "MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) "
        "CREATE (a)-[:KNOWS]->(b) RETURN a, b",
    )
    check("KNOWS エッジ作成", resp.get("type") == "result", str(resp))

    resp = run_query(
        client,
        "MATCH (a:Person {name: 'Alice'}), (c:Company {name: 'Acme'}) "
        "CREATE (a)-[:WORKS_AT]->(c) RETURN a, c",
    )
    check("WORKS_AT エッジ作成", resp.get("type") == "result", str(resp))


def test_match(client: MaharitClient):
    section("MATCH 検索")

    resp = run_query(client, "MATCH (n:Person) RETURN n")
    check("Person ノード取得", resp.get("type") == "result", str(resp))
    check("2件以上ある", len(resp.get("rows", [])) >= 2,
          f"rows={len(resp.get('rows', []))}")

    resp = run_query(client, "MATCH (n:Person) WHERE n.age > 26 RETURN n")
    check("WHERE age > 26 フィルタ", resp.get("type") == "result", str(resp))
    rows = resp.get("rows", [])
    check("Alice のみ返る", len(rows) == 1, f"rows={len(rows)}")


def test_relationship_match(client: MaharitClient):
    section("MATCH リレーションシップ")

    resp = run_query(
        client,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b",
    )
    check("KNOWS トラバーサル", resp.get("type") == "result", str(resp))
    check("1件返る", len(resp.get("rows", [])) >= 1,
          f"rows={len(resp.get('rows', []))}")

    resp = run_query(
        client,
        "MATCH (a:Person)-[:WORKS_AT]->(c:Company) RETURN a.name, c.name",
    )
    check("WORKS_AT トラバーサル", resp.get("type") == "result", str(resp))


def test_set(client: MaharitClient):
    section("SET 更新")

    resp = run_query(
        client,
        "MATCH (n:Person {name: 'Bob'}) SET n.age = 26 RETURN n",
    )
    check("Bob の age を更新", resp.get("type") == "result", str(resp))

    resp = run_query(client, "MATCH (n:Person {name: 'Bob'}) RETURN n.age")
    rows = resp.get("rows", [])
    age_val = rows[0].get("n.age") if rows else None
    check("更新後の age が 26", age_val == "26", f"age={age_val}")


def test_delete(client: MaharitClient):
    section("DELETE 削除")

    resp = run_query(
        client,
        "MATCH (n:Person {name: 'Bob'}) DETACH DELETE n",
    )
    check("Bob を削除", resp.get("type") == "result", str(resp))

    resp = run_query(client, "MATCH (n:Person {name: 'Bob'}) RETURN n")
    check("削除後は見つからない", len(resp.get("rows", [])) == 0,
          f"rows={len(resp.get('rows', []))}")


def test_transaction_commit(client: MaharitClient):
    section("トランザクション COMMIT")

    resp = client.send({"type": "begin"})
    check("BEGIN", resp.get("type") == "transactionBegun", str(resp))
    tx_id = resp.get("txId")

    run_query(client, "CREATE (t:TxTest {val: 'committed'})")

    resp = client.send({"type": "commit", "txId": tx_id})
    check("COMMIT", resp.get("type") == "committed", str(resp))

    resp = run_query(client, "MATCH (n:TxTest) RETURN n")
    check("COMMIT 後にデータが残る", len(resp.get("rows", [])) >= 1,
          f"rows={len(resp.get('rows', []))}")


def test_transaction_rollback(client: MaharitClient):
    section("トランザクション ROLLBACK")

    resp = run_query(client, "MATCH (n:TxTest) RETURN n")
    before = len(resp.get("rows", []))

    resp = client.send({"type": "begin"})
    check("BEGIN", resp.get("type") == "transactionBegun", str(resp))
    tx_id = resp.get("txId")

    client.send({"type": "query", "query": "CREATE (t:TxTest {val: 'will_rollback'})", "txId": tx_id})

    resp = client.send({"type": "rollback", "txId": tx_id})
    check("ROLLBACK", resp.get("type") == "rolledBack", str(resp))

    resp = run_query(client, "MATCH (n:TxTest) RETURN n")
    after = len(resp.get("rows", []))
    check("ROLLBACK 後に件数が変わらない", after == before,
          f"before={before}, after={after}")


def test_stream_query(client: MaharitClient):
    section("ストリーミングクエリ")

    # 複数ノードを作成してからストリームで取得
    run_query(client, "CREATE (:StreamTest {i: 1})")
    run_query(client, "CREATE (:StreamTest {i: 2})")
    run_query(client, "CREATE (:StreamTest {i: 3})")

    messages = client.send({
        "type": "streamQuery",
        "query": "MATCH (n:StreamTest) RETURN n",
        "chunkSize": 2,
    })

    types = [m.get("type") for m in messages]
    check("streamStart が含まれる", "streamStart" in types, str(types))
    check("streamEnd が含まれる", "streamEnd" in types, str(types))

    chunks = [m for m in messages if m.get("type") == "streamChunk"]
    total_rows = sum(len(c.get("rows", [])) for c in chunks)
    check("3件のデータが返る", total_rows >= 3, f"total_rows={total_rows}")


def test_error_handling(client: MaharitClient):
    section("エラーハンドリング")

    resp = run_query(client, "INVALID QUERY !!!")
    check("不正クエリでエラーが返る", resp.get("type") == "error", str(resp))


def test_cleanup(client: MaharitClient):
    section("クリーンアップ")

    for label in ("TxTest", "StreamTest", "Company"):
        resp = run_query(client, f"MATCH (n:{label}) DETACH DELETE n")
        check(f"{label} 削除", resp.get("type") == "result", str(resp))

    resp = run_query(client, "MATCH (n:Person) DETACH DELETE n")
    check("Person 削除", resp.get("type") == "result", str(resp))

    resp = run_query(client, "MATCH (n) RETURN n")
    check("全ノード削除済み", len(resp.get("rows", [])) == 0,
          f"残り={len(resp.get('rows', []))}")


# ── エントリポイント ──────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description="MaharitDB Smoke Test")
    parser.add_argument("--host", default="localhost")
    parser.add_argument("--port", type=int, default=7687)
    args = parser.parse_args()

    print(f"\n{BOLD}MaharitDB Smoke Test{RESET}")
    print(f"接続先: {args.host}:{args.port}")
    print("─" * 40)

    try:
        client = MaharitClient(args.host, args.port)
    except Exception as e:
        print(f"{RED}接続失敗: {e}{RESET}")
        sys.exit(1)

    try:
        test_ping(client)
        test_stats(client)
        test_create_nodes(client)
        test_create_edges(client)
        test_match(client)
        test_relationship_match(client)
        test_set(client)
        test_delete(client)
        test_transaction_commit(client)
        test_transaction_rollback(client)
        test_stream_query(client)
        test_error_handling(client)
        test_cleanup(client)
    finally:
        client.close()

    print(f"\n{'─' * 40}")
    print(f"{BOLD}結果: {GREEN}{passed} passed{RESET}", end="")
    if failed:
        print(f", {RED}{failed} failed{RESET}")
    else:
        print()

    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
