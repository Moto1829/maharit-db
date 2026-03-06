"""MaharitDB wire protocol helpers.

Protocol: 4-byte big-endian u32 length prefix followed by JSON payload.
"""

import json
import struct
import socket as _socket
from typing import Any


_LENGTH_PREFIX_FORMAT = ">I"  # big-endian unsigned 32-bit int
_LENGTH_PREFIX_SIZE = struct.calcsize(_LENGTH_PREFIX_FORMAT)


def encode_message(payload: dict) -> bytes:
    """Encode a request dict into length-prefixed JSON bytes."""
    data = json.dumps(payload).encode("utf-8")
    prefix = struct.pack(_LENGTH_PREFIX_FORMAT, len(data))
    return prefix + data


def send_message(sock: _socket.socket, payload: dict) -> None:
    """Send a single length-prefixed JSON message over a socket."""
    sock.sendall(encode_message(payload))


def recv_message(sock: _socket.socket) -> dict:
    """Receive a single length-prefixed JSON message from a socket."""
    # Read 4-byte length prefix
    prefix = _recv_exactly(sock, _LENGTH_PREFIX_SIZE)
    (length,) = struct.unpack(_LENGTH_PREFIX_FORMAT, prefix)
    # Read the JSON payload
    data = _recv_exactly(sock, length)
    return json.loads(data.decode("utf-8"))


def _recv_exactly(sock: _socket.socket, n: int) -> bytes:
    """Read exactly n bytes from a socket."""
    buf = bytearray()
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("Server closed the connection unexpectedly")
        buf.extend(chunk)
    return bytes(buf)
