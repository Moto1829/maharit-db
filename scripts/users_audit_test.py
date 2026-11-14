#!/usr/bin/env python3
"""
MaharitDB ユーザーの永続化（--users-file）と監査ログ（--audit-log）の E2E テスト

サーバーを --require-auth --users-file --audit-log 付きで起動してユーザーを作り、
再起動後もそのユーザーでログインできること、監査ログに記録されることを確かめる。
サーバーは自分で起動・停止する（事前の起動は不要）。

使い方:
  python3 scripts/users_audit_test.py
  python3 scripts/users_audit_test.py --binary ./target/release/maharit --port 7698
"""

import argparse
import json
import os
import shutil
import signal
import stat
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from lib.client import MaharitClient  # noqa: E402
from lib.reporting import check, section, summarize  # noqa: E402
from persistence_test import find_binary, start_server, stop_server, wait_for_server  # noqa: E402

HOST = "127.0.0.1"


def login(c: MaharitClient, user: str, pw: str) -> dict:
    return c.send({"type": "login", "username": user, "password": pw})


def query(c: MaharitClient, token: str, q: str) -> dict:
    return c.send({"type": "query", "query": q, "sessionToken": token})


def audit_events(path: str) -> list[dict]:
    if not os.path.exists(path):
        return []
    with open(path) as f:
        return [json.loads(line) for line in f if line.strip()]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", default=None)
    ap.add_argument("--port", type=int, default=7698)
    args = ap.parse_args()
    binary = args.binary or find_binary()

    work = tempfile.mkdtemp(prefix="maharit-users-")
    data = os.path.join(work, "db.mgdb")
    users = os.path.join(work, "users.json")
    audit = os.path.join(work, "audit.log")
    flags = ["--require-auth", "--users-file", users, "--audit-log", audit]

    try:
        # ── 1 回目: ファイルが無いので --admin-password で作る ──
        section("初回起動（users.json を作成）")
        proc = start_server(binary, args.port, data, [*flags, "--admin-password", "first-pw"])
        check("サーバーが起動する", wait_for_server(HOST, args.port))
        check("users.json が作られる", os.path.exists(users))
        if os.name == "posix" and os.path.exists(users):
            mode = stat.S_IMODE(os.stat(users).st_mode)
            check("users.json の権限は 0600", mode == 0o600, oct(mode))

        with MaharitClient(HOST, args.port) as c:
            r = login(c, "admin", "wrong")
            check("誤ったパスワードは拒否", r.get("type") == "authError", str(r))
            r = login(c, "admin", "first-pw")
            check("admin でログインできる", r.get("type") == "loggedIn", str(r))
            admin = r.get("sessionToken", "")
            r = query(c, admin, "CREATE USER alice SET PASSWORD 'alice-pw' SET ROLE 'reader'")
            check("CREATE USER alice", r.get("type") == "result", str(r))

            r = login(c, "alice", "alice-pw")
            alice = r.get("sessionToken", "")
            r = query(c, alice, "CREATE (:X)")
            check("reader の書き込みは拒否", r.get("type") == "authError", str(r))
            r = query(c, alice, "CREATE USER bob SET PASSWORD 'b' SET ROLE 'reader'")
            check("reader のユーザー管理は拒否", r.get("type") == "authError", str(r))

        stop_server(proc, signal.SIGTERM)

        # ── 2 回目: ファイルから読み込む（--admin-password は無視される） ──
        section("再起動（users.json から読み込む）")
        proc = start_server(binary, args.port, data, [*flags, "--admin-password", "other-pw"])
        check("サーバーが起動する", wait_for_server(HOST, args.port))
        with MaharitClient(HOST, args.port) as c:
            r = login(c, "alice", "alice-pw")
            check("再起動後も alice でログインできる", r.get("type") == "loggedIn", str(r))
            r = login(c, "admin", "other-pw")
            check("ファイルがあるとき --admin-password は使われない", r.get("type") == "authError", str(r))
            r = login(c, "admin", "first-pw")
            check("admin は最初のパスワードのまま", r.get("type") == "loggedIn", str(r))
            admin = r.get("sessionToken", "")
            r = query(c, admin, "DROP USER alice")
            check("DROP USER alice", r.get("type") == "result", str(r))
        stop_server(proc, signal.SIGTERM)

        # ── 3 回目: パスワード無しでも起動でき、削除が残っている ──
        section("再起動（--admin-password なし）")
        proc = start_server(binary, args.port, data, flags)
        check("--admin-password なしで起動する", wait_for_server(HOST, args.port))
        with MaharitClient(HOST, args.port) as c:
            r = login(c, "alice", "alice-pw")
            check("削除したユーザーは復活しない", r.get("type") == "authError", str(r))
        stop_server(proc, signal.SIGTERM)

        # ── 監査ログ ──
        section("監査ログ")
        events = audit_events(audit)
        kinds = [(e.get("event"), e.get("username"), e.get("by") or e.get("action")) for e in events]
        for expected in [
            ("login_failure", "admin", None),
            ("login_success", "admin", None),
            ("user_created", "alice", "admin"),
            ("permission_denied", "alice", "write"),
            ("permission_denied", "alice", "manage_users"),
            ("user_deleted", "alice", "admin"),
        ]:
            check(f"監査ログに {expected}", expected in kinds, str(kinds))
        check("パスワードが監査ログに出ない", "alice-pw" not in open(audit).read())
    finally:
        shutil.rmtree(work, ignore_errors=True)

    return summarize()


if __name__ == "__main__":
    sys.exit(main())
