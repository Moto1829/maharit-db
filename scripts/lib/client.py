"""MaharitDB の TCP プロトコルクライアント。

プロトコル: 4 バイト長プレフィックス + JSON ペイロード（big-endian）。

`MaharitClient` は同期 socket でリクエスト/レスポンスを 1 対 1 でやり取りする。
`streamQuery` のようにサーバーから複数メッセージが返るリクエストでは
`send_stream` を使って逐次受信できる。
"""

from __future__ import annotations

import json
import socket
import struct
from typing import Iterator


class MaharitClient:
    """同期 socket ベースの MaharitDB クライアント。"""

    def __init__(self, host: str, port: int, timeout: float = 10.0):
        self.host = host
        self.port = port
        self.sock = socket.create_connection((host, port), timeout=timeout)

    # ── 高レベル API ────────────────────────────────────────────────────────

    def query(self, cypher: str) -> dict:
        """単発のクエリを投げて 1 つのレスポンスを返す。"""
        return self.send({"type": "query", "query": cypher})

    def stream_query(self, cypher: str) -> list[dict]:
        """streamQuery を発行し、streamEnd / error までのメッセージを集めて返す。"""
        return self.send({"type": "streamQuery", "query": cypher})

    def ping(self) -> bool:
        """ping を投げて pong が返るかを bool で返す。例外時は False。"""
        try:
            resp = self.send({"type": "ping"})
            return resp.get("type") == "pong"
        except Exception:
            return False

    # ── 低レベル API ────────────────────────────────────────────────────────

    def send(self, request: dict) -> dict | list[dict]:
        """JSON リクエストを送り、種別に応じて 1 件 or リストでレスポンスを返す。"""
        data = json.dumps(request).encode()
        self.sock.sendall(struct.pack(">I", len(data)) + data)

        if request.get("type") == "streamQuery":
            return list(self._recv_stream())
        return self._recv_one()

    def iter_stream(self, cypher: str) -> Iterator[dict]:
        """ストリーミングクエリのメッセージを逐次 yield する（大規模結果向け）。"""
        data = json.dumps({"type": "streamQuery", "query": cypher}).encode()
        self.sock.sendall(struct.pack(">I", len(data)) + data)
        yield from self._recv_stream()

    def close(self):
        """切断リクエストを送ってから socket を閉じる（例外は無視）。"""
        try:
            self.send({"type": "disconnect"})
        except Exception:
            pass
        try:
            self.sock.close()
        except Exception:
            pass

    # ── 内部: 受信 ──────────────────────────────────────────────────────────

    def _recv_one(self) -> dict:
        raw_len = self._recv_exactly(4)
        length = struct.unpack(">I", raw_len)[0]
        payload = self._recv_exactly(length)
        return json.loads(payload)

    def _recv_stream(self) -> Iterator[dict]:
        """streamEnd または error が来るまでメッセージを yield する。"""
        while True:
            msg = self._recv_one()
            yield msg
            if msg.get("type") in ("streamEnd", "error"):
                break

    def _recv_exactly(self, n: int) -> bytes:
        buf = b""
        while len(buf) < n:
            chunk = self.sock.recv(n - len(buf))
            if not chunk:
                raise ConnectionError("Connection closed by server")
            buf += chunk
        return buf

    # ── context manager サポート ────────────────────────────────────────────

    def __enter__(self) -> "MaharitClient":
        return self

    def __exit__(self, exc_type, exc, tb):
        self.close()
