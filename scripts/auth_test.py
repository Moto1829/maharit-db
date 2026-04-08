#!/usr/bin/env python3
"""
MaharitDB 認証・認可 E2E テスト (Task 20 で実装済みの機能)

CREATE USER / DROP USER / ALTER USER / SHOW USERS クエリ構文を通じた
ユーザー管理機能の動作を検証する。

注意: このスクリプトは認証が有効化されたサーバーインスタンスを想定する。
     標準の docker compose up -d maharit-server では認証が無効のため、
     認証が有効な設定で起動したサーバーに対して実行すること。

使い方:
  python3 scripts/auth_test.py
  python3 scripts/auth_test.py --host localhost --port 7687
"""

import argparse
import json
import socket
import struct
import subprocess
import sys

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
        self.sock = socket.create_connection((host, port), timeout=timeout)

    def send(self, request: dict) -> dict:
        data = json.dumps(request).encode()
        self.sock.sendall(struct.pack(">I", len(data)) + data)
        return self._recv_one()

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


# ── テストヘルパー ────────────────────────────────────────────────────────────

passed = 0
failed = 0
errors = []
skipped = 0


def check(name: str, condition: bool, detail: str = ""):
    global passed, failed
    if condition:
        passed += 1
        print(f"  {GREEN}✓{RESET} {name}")
    else:
        failed += 1
        detail_str = f" — {detail}" if detail else ""
        print(f"  {RED}✗{RESET} {name}{detail_str}")
        errors.append(f"{name}{detail_str}")


def check_skip(name: str, condition: bool, detail: str = "", skip_reason: str = ""):
    """条件がスキップ相当の場合は SKIP として記録する。"""
    global passed, failed, skipped
    if skip_reason:
        skipped += 1
        print(f"  {YELLOW}~{RESET} {name} (SKIP: {skip_reason})")
        return
    check(name, condition, detail)


def section(title: str):
    print(f"\n{CYAN}{BOLD}▶ {title}{RESET}")


def run_query(client: MaharitClient, query: str) -> dict:
    return client.send({"type": "query", "query": query})


def setup(client: MaharitClient):
    """テスト前にテスト用ユーザーを削除しておく。"""
    for username in ["testuser1", "testuser2", "readonly_user", "admin_test"]:
        run_query(client, f"DROP USER {username} IF EXISTS")


def teardown(client: MaharitClient):
    """テスト後にテスト用ユーザーを削除する。"""
    for username in ["testuser1", "testuser2", "readonly_user", "admin_test"]:
        run_query(client, f"DROP USER {username} IF EXISTS")


# ── テストスイート ────────────────────────────────────────────────────────────

def test_create_user(client: MaharitClient):
    section("CREATE USER")

    # 基本的なユーザー作成
    resp = run_query(
        client,
        "CREATE USER testuser1 SET PASSWORD 'password123' SET ROLE 'reader'",
    )
    check("CREATE USER testuser1 が成功", resp.get("type") == "result", str(resp))

    # SHOW USERS で確認
    resp = run_query(client, "SHOW USERS")
    check("SHOW USERS でエラーなし", resp.get("type") == "result", str(resp))
    rows = resp.get("rows", [])
    usernames = set()
    for row in rows:
        for v in row.values():
            usernames.add(str(v))
    # testuser1 がユーザー一覧に含まれているか
    check(
        "SHOW USERS に testuser1 が含まれる",
        any("testuser1" in str(r) for r in rows),
        f"rows={rows}",
    )

    # 管理者ロールでユーザー作成
    resp = run_query(
        client,
        "CREATE USER admin_test SET PASSWORD 'adminpass' SET ROLE 'admin'",
    )
    check("CREATE USER admin_test (role=admin) が成功", resp.get("type") == "result", str(resp))

    # 既存ユーザー名で CREATE USER はエラー
    resp = run_query(
        client,
        "CREATE USER testuser1 SET PASSWORD 'other_password' SET ROLE 'reader'",
    )
    check(
        "重複 CREATE USER はエラーが返る",
        resp.get("type") == "error",
        str(resp),
    )


def test_drop_user(client: MaharitClient):
    section("DROP USER")

    # ユーザーを作成してから削除
    run_query(client, "CREATE USER testuser2 SET PASSWORD 'temp_pass' SET ROLE 'reader'")

    resp = run_query(client, "DROP USER testuser2")
    check("DROP USER testuser2 が成功", resp.get("type") == "result", str(resp))

    # SHOW USERS で消えていることを確認
    resp = run_query(client, "SHOW USERS")
    rows = resp.get("rows", [])
    check(
        "DROP USER 後に testuser2 が SHOW USERS に含まれない",
        not any("testuser2" in str(r) for r in rows),
        f"rows={rows}",
    )

    # 存在しないユーザーの DROP はエラー
    resp = run_query(client, "DROP USER testuser2")
    check(
        "存在しないユーザーの DROP USER はエラー",
        resp.get("type") == "error",
        str(resp),
    )


def test_alter_user(client: MaharitClient):
    section("ALTER USER")

    # ユーザー作成
    run_query(client, "CREATE USER readonly_user SET PASSWORD 'readpass' SET ROLE 'reader'")

    # パスワード変更
    resp = run_query(
        client,
        "ALTER USER readonly_user SET PASSWORD 'newpass123'",
    )
    check("ALTER USER SET PASSWORD が成功", resp.get("type") == "result", str(resp))

    # ロール変更
    resp = run_query(
        client,
        "ALTER USER readonly_user SET ROLE 'writer'",
    )
    check("ALTER USER SET ROLE が成功", resp.get("type") == "result", str(resp))

    # 存在しないユーザーの ALTER はエラー
    resp = run_query(
        client,
        "ALTER USER nonexistent_user SET PASSWORD 'pass'",
    )
    check(
        "存在しないユーザーの ALTER USER はエラー",
        resp.get("type") == "error",
        str(resp),
    )


def test_show_users(client: MaharitClient):
    section("SHOW USERS")

    # 複数ユーザー作成後に SHOW USERS
    run_query(client, "CREATE USER testuser1 SET PASSWORD 'pass1' SET ROLE 'reader'")
    run_query(client, "CREATE USER admin_test SET PASSWORD 'pass2' SET ROLE 'admin'")

    resp = run_query(client, "SHOW USERS")
    check("SHOW USERS でエラーなし", resp.get("type") == "result", str(resp))

    rows = resp.get("rows", [])
    check(
        "SHOW USERS で複数ユーザーが返る",
        len(rows) >= 2,
        f"rows={len(rows)}",
    )

    # admin ユーザーが含まれているはず（デフォルト）
    check(
        "SHOW USERS に admin が含まれる",
        any("admin" in str(r) for r in rows),
        f"rows={rows}",
    )


def test_rbac_role_enforcement(client: MaharitClient):
    """
    ロールベースアクセス制御の検証。
    サーバーが認証を強制する設定の場合のみ意味を持つ。
    標準構成では認証が無効なため、このセクションはスキップまたは制限付きテスト。
    """
    section("RBAC（ロールベースアクセス制御）")

    # reader ロールのユーザー作成
    run_query(client, "CREATE USER readonly_user SET PASSWORD 'readpass' SET ROLE 'reader'")

    # ユーザーが作成されたことを確認
    resp = run_query(client, "SHOW USERS")
    rows = resp.get("rows", [])
    check(
        "reader ロールのユーザーが作成された",
        any("readonly_user" in str(r) for r in rows),
        f"rows={rows}",
    )

    # NOTE: 実際のアクセス制御テストは認証が有効なサーバーが必要。
    # 現在のプロトコルには login/auth メッセージタイプがないため、
    # 別接続での権限テストはサーバー設定に依存する。
    check_skip(
        "reader ロールでの書き込みが拒否される（認証有効時のみ）",
        False,
        skip_reason="認証が有効なサーバー構成が必要",
    )
    check_skip(
        "admin ロールでの書き込みが許可される（認証有効時のみ）",
        False,
        skip_reason="認証が有効なサーバー構成が必要",
    )


# ── エントリポイント ──────────────────────────────────────────────────────────

def check_docker_compose() -> None:
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
        print("  docker compose -f docker-compose.replication.yml down")
        print("  docker compose up -d maharit-server\n")
        sys.exit(1)

    print(f"\n{RED}エラー: maharit-db-server コンテナが起動していません。{RESET}")
    print("  docker compose up -d maharit-server\n")
    sys.exit(1)


def main():
    parser = argparse.ArgumentParser(description="MaharitDB 認証・認可 E2E テスト")
    parser.add_argument("--host", default="localhost")
    parser.add_argument("--port", type=int, default=7687)
    args = parser.parse_args()
    check_docker_compose()

    print(f"\n{BOLD}MaharitDB 認証・認可 E2E テスト{RESET}")
    print(f"接続先: {args.host}:{args.port}")
    print(f"{YELLOW}注意: RBAC テストはサーバーの認証設定に依存します。{RESET}")
    print("─" * 50)

    try:
        client = MaharitClient(args.host, args.port)
    except Exception as e:
        print(f"{RED}接続失敗: {e}{RESET}")
        sys.exit(1)

    setup(client)

    try:
        test_create_user(client)
        test_drop_user(client)
        test_alter_user(client)
        test_show_users(client)
        test_rbac_role_enforcement(client)
    finally:
        teardown(client)
        client.close()

    print(f"\n{'─' * 50}")
    print(f"{BOLD}結果: {GREEN}{passed} passed{RESET}", end="")
    if failed:
        print(f", {RED}{failed} failed{RESET}", end="")
    if skipped:
        print(f", {YELLOW}{skipped} skipped{RESET}", end="")
    print()

    if errors:
        print(f"\n失敗したテスト:")
        for e in errors:
            print(f"  {RED}✗{RESET} {e}")

    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
