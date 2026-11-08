#!/usr/bin/env python3
"""
MaharitDB TLS テスト

自己署名証明書を作り、`--tls-cert` / `--tls-key` 付きでサーバーを起動して、
Python クライアント（同期・非同期）が TLS で接続できること、平文や
信頼していない証明書では接続できないこと、設定の誤りで起動を拒否することを
検証する。証明書の作成に openssl コマンドを使う。

使い方:
  python3 scripts/tls_test.py
  python3 scripts/tls_test.py --binary ./target/release/maharit --port 7697
"""

import argparse
import asyncio
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "clients", "python"))

from lib.client import MaharitClient  # noqa: E402
from lib.reporting import check, section, summarize  # noqa: E402
from maharit import AsyncClient, Client  # noqa: E402
from maharit import ConnectionError as MaharitConnectionError  # noqa: E402


def find_binary() -> str:
    for path in (
        os.path.join(ROOT, "target", "release", "maharit"),
        os.path.join(ROOT, "target", "debug", "maharit"),
    ):
        if os.path.isfile(path):
            return path
    raise FileNotFoundError("maharit バイナリが見つかりません。先に `cargo build` を実行してください。")


def make_cert(directory: str, name: str) -> tuple[str, str]:
    """localhost / 127.0.0.1 用の自己署名サーバー証明書（CA:FALSE）を作る。"""
    cert = os.path.join(directory, f"{name}.crt")
    key = os.path.join(directory, f"{name}.key")
    subprocess.run(
        [
            "openssl", "req", "-x509", "-newkey", "ec",
            "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
            "-keyout", key, "-out", cert, "-days", "1", "-subj", "/CN=localhost",
            "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1",
            "-addext", "basicConstraints=critical,CA:FALSE",
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return cert, key


def start_server(binary: str, port: int, data: str, extra: list[str]) -> subprocess.Popen:
    return subprocess.Popen(
        [binary, "server", "--port", str(port), "--data", data, "--wal", "off", *extra],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )


def wait_tls_ready(port: int, ca_file: str, timeout: float = 10.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with Client.connect(f"127.0.0.1:{port}", timeout=1.0, max_retries=0, ca_file=ca_file) as c:
                if c.ping():
                    return True
        except Exception:
            pass
        time.sleep(0.1)
    return False


def stop(proc: subprocess.Popen) -> str:
    if proc.poll() is None:
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
    return proc.stdout.read() if proc.stdout else ""


def test_tls_connections(binary: str, port: int, tmp: str) -> None:
    section("TLS で接続する")
    cert, key = make_cert(tmp, "server")
    other_cert, _ = make_cert(tmp, "other")
    proc = start_server(
        binary, port, os.path.join(tmp, "tls.db"), ["--tls-cert", cert, "--tls-key", key]
    )
    try:
        ready = wait_tls_ready(port, cert)
        check("TLS サーバー起動・ping", ready)
        if not ready:
            return
        addr = f"127.0.0.1:{port}"

        with Client.connect(addr, ca_file=cert) as c:
            c.execute("CREATE (:Secret {name: $name})", params={"name": "tls"})
            rows = list(c.query("MATCH (s:Secret) RETURN s.name AS name"))
            check("同期クライアント: 書き込みと読み出し", rows == [{"name": "tls"}], str(rows))

        async def async_roundtrip() -> list:
            client = await AsyncClient.connect(addr, ca_file=cert)
            try:
                return list(await client.query("MATCH (s:Secret) RETURN s.name AS name"))
            finally:
                await client.close()

        rows = asyncio.run(async_roundtrip())
        check("非同期クライアント: 読み出し", rows == [{"name": "tls"}], str(rows))

        # 平文のクライアントには応答しない
        answered = False
        try:
            plain = MaharitClient("127.0.0.1", port, timeout=3.0)
            answered = plain.ping()
            plain.close()
        except Exception:
            pass
        check("平文の接続には応答しない", not answered)

        # 信頼していない証明書・システムの CA ストアでは検証に失敗する
        for label, kwargs in (
            ("別の証明書を信頼したクライアントは接続できない", {"ca_file": other_cert}),
            ("システムの CA ストアだけでは自己署名証明書を信頼しない", {"ssl": True}),
        ):
            rejected = False
            try:
                Client.connect(addr, timeout=3.0, max_retries=0, **kwargs).close()
            except MaharitConnectionError:
                rejected = True
            check(label, rejected)

        check("TLS 接続後もサーバーが動作中", proc.poll() is None)
    finally:
        output = stop(proc)
    check("起動ログに TLS が出る", "(TLS)" in output, " / ".join(output.strip().splitlines()[:3]))


def test_misconfiguration(binary: str, port: int, tmp: str) -> None:
    section("設定の誤りでは起動しない")
    cert, key = make_cert(tmp, "misc")
    cases = [
        ("--tls-cert だけ指定", ["--tls-cert", cert]),
        ("存在しない鍵ファイル", ["--tls-cert", cert, "--tls-key", os.path.join(tmp, "missing.key")]),
        ("証明書と鍵を取り違える", ["--tls-cert", key, "--tls-key", cert]),
        ("--tls-min-version が不正", ["--tls-cert", cert, "--tls-key", key, "--tls-min-version", "1.1"]),
    ]
    for label, extra in cases:
        proc = start_server(binary, port, os.path.join(tmp, "misc.db"), extra)
        try:
            code = proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            code = None
        output = stop(proc)
        check(f"{label}: 異常終了する", code not in (None, 0), output.strip()[-200:])


def main() -> None:
    parser = argparse.ArgumentParser(description="MaharitDB TLS テスト")
    parser.add_argument("--binary", default=None)
    parser.add_argument("--port", type=int, default=7697)
    args = parser.parse_args()

    if shutil.which("openssl") is None:
        print("openssl コマンドが見つからないため TLS テストを実行できません")
        sys.exit(1)
    binary = args.binary or find_binary()
    print(f"バイナリ: {binary}\nポート: {args.port}\n")

    tmp = tempfile.mkdtemp(prefix="maharit_tls_test_")
    try:
        test_tls_connections(binary, args.port, tmp)
        test_misconfiguration(binary, args.port, tmp)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    sys.exit(summarize())


if __name__ == "__main__":
    main()
