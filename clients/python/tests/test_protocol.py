"""Tests for the wire protocol helpers."""

import json
import socket
import struct
import threading

import pytest

from maharit.protocol import encode_message, send_message, recv_message


def test_encode_message_basic():
    msg = {"type": "ping"}
    encoded = encode_message(msg)
    assert len(encoded) >= 4
    (length,) = struct.unpack(">I", encoded[:4])
    payload = json.loads(encoded[4:4 + length].decode("utf-8"))
    assert payload == msg


def test_encode_message_roundtrip():
    msg = {"type": "query", "query": "MATCH (n) RETURN n", "txId": 42}
    encoded = encode_message(msg)
    (length,) = struct.unpack(">I", encoded[:4])
    assert length == len(encoded) - 4
    decoded = json.loads(encoded[4:].decode("utf-8"))
    assert decoded == msg


def test_send_recv_message_over_socketpair():
    """send_message and recv_message work over a real socket pair."""
    a, b = socket.socketpair()
    try:
        msg = {"type": "result", "rows": [{"name": "Alice"}, {"name": "Bob"}]}
        send_message(a, msg)
        received = recv_message(b)
        assert received == msg
    finally:
        a.close()
        b.close()


def test_recv_large_message():
    """Protocol handles messages with many rows (send in thread to avoid buffer deadlock)."""
    a, b = socket.socketpair()
    rows = [{"id": i, "val": "x" * 100} for i in range(200)]
    msg = {"type": "result", "rows": rows}

    def sender():
        send_message(a, msg)
        a.close()

    t = threading.Thread(target=sender, daemon=True)
    t.start()
    try:
        received = recv_message(b)
        assert received["rows"] == rows
    finally:
        b.close()
        t.join(timeout=5)


def test_recv_raises_on_closed_connection():
    a, b = socket.socketpair()
    a.close()
    with pytest.raises((ConnectionError, OSError)):
        recv_message(b)
    b.close()
