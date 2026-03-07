"""Synchronous MaharitDB client."""

import socket
import time
from contextlib import contextmanager
from typing import Any, Dict, Generator, Iterator, List, Optional

from .exceptions import ConnectionError, MaharitError, QueryError, TransactionError
from .protocol import recv_message, send_message


class QueryResult:
    """Result of a query execution.

    Supports iteration, indexing, and optional pandas DataFrame conversion.
    """

    def __init__(self, rows: List[Dict[str, str]]) -> None:
        self._rows = rows

    def __iter__(self) -> Iterator[Dict[str, str]]:
        return iter(self._rows)

    def __len__(self) -> int:
        return len(self._rows)

    def __getitem__(self, index: int) -> Dict[str, str]:
        return self._rows[index]

    def to_dataframe(self):
        """Convert the result to a pandas DataFrame.

        Requires pandas to be installed.
        """
        try:
            import pandas as pd
        except ImportError as e:
            raise ImportError(
                "pandas is required for to_dataframe(). "
                "Install it with: pip install pandas"
            ) from e
        return pd.DataFrame(self._rows)

    def __repr__(self) -> str:
        return f"QueryResult({len(self._rows)} rows)"


class Transaction:
    """Represents an active database transaction."""

    def __init__(self, client: "Client", tx_id: int) -> None:
        self._client = client
        self._tx_id = tx_id
        self._closed = False

    @property
    def tx_id(self) -> int:
        return self._tx_id

    def execute(self, query: str) -> QueryResult:
        """Execute a query within this transaction."""
        if self._closed:
            raise TransactionError("Transaction is already closed")
        return self._client._execute_query(query, tx_id=self._tx_id)

    def commit(self) -> None:
        """Commit the transaction."""
        if self._closed:
            raise TransactionError("Transaction is already closed")
        self._client._send({"type": "commit", "txId": self._tx_id})
        response = self._client._recv()
        self._closed = True
        if response.get("type") == "error":
            raise TransactionError(response.get("message", "Commit failed"))
        if response.get("type") != "committed":
            raise TransactionError(f"Unexpected response: {response}")

    def rollback(self) -> None:
        """Rollback the transaction."""
        if self._closed:
            return
        try:
            self._client._send({"type": "rollback", "txId": self._tx_id})
            self._client._recv()
        except Exception:
            pass
        finally:
            self._closed = True

    def __enter__(self) -> "Transaction":
        return self

    def __exit__(self, exc_type, exc_val, exc_tb) -> None:
        if exc_type is None:
            self.commit()
        else:
            self.rollback()

    def __repr__(self) -> str:
        return f"Transaction(id={self._tx_id}, closed={self._closed})"


class Client:
    """Synchronous MaharitDB client.

    Example::

        with Client.connect("localhost:7687") as client:
            client.execute("CREATE (n:Person {name: 'Alice'})")
            result = client.query("MATCH (n:Person) RETURN n.name")
            for row in result:
                print(row["n.name"])
    """

    def __init__(
        self,
        host: str,
        port: int,
        timeout: float = 30.0,
        max_retries: int = 3,
        retry_delay: float = 1.0,
    ) -> None:
        self._host = host
        self._port = port
        self._timeout = timeout
        self._max_retries = max_retries
        self._retry_delay = retry_delay
        self._sock: Optional[socket.socket] = None

    @classmethod
    def connect(
        cls,
        address: str,
        timeout: float = 30.0,
        max_retries: int = 3,
        retry_delay: float = 1.0,
    ) -> "Client":
        """Connect to a MaharitDB server.

        Args:
            address: Server address in the form ``host:port``.
            timeout: Socket timeout in seconds (default 30).
            max_retries: Number of automatic reconnection attempts on failure
                (default 3). Set to 0 to disable auto-reconnect.
            retry_delay: Base delay in seconds between retry attempts. Each
                attempt waits ``retry_delay * attempt`` seconds (default 1.0).

        Returns:
            A connected :class:`Client` instance.
        """
        host, _, port_str = address.rpartition(":")
        if not host:
            host = "localhost"
        port = int(port_str) if port_str else 7687
        client = cls(host, port, timeout, max_retries, retry_delay)
        client._connect()
        return client

    def _connect(self) -> None:
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.settimeout(self._timeout)
        try:
            sock.connect((self._host, self._port))
        except OSError as e:
            sock.close()
            raise ConnectionError(f"Cannot connect to {self._host}:{self._port}: {e}") from e
        self._sock = sock

    def _reconnect(self) -> None:
        if self._sock:
            try:
                self._sock.close()
            except Exception:
                pass
        self._connect()

    def _send(self, payload: dict) -> None:
        if self._sock is None:
            raise ConnectionError("Not connected")
        for attempt in range(self._max_retries + 1):
            try:
                send_message(self._sock, payload)
                return
            except OSError as e:
                if attempt == self._max_retries:
                    raise ConnectionError(f"Send failed after {attempt + 1} attempt(s): {e}") from e
                time.sleep(self._retry_delay * (attempt + 1))
                try:
                    self._reconnect()
                except Exception:
                    pass

    def _recv(self) -> dict:
        if self._sock is None:
            raise ConnectionError("Not connected")
        try:
            return recv_message(self._sock)
        except OSError as e:
            raise ConnectionError(f"Receive failed: {e}") from e

    def ping(self) -> bool:
        """Ping the server to check connectivity.

        Returns:
            ``True`` if the server responds with pong.
        """
        self._send({"type": "ping"})
        response = self._recv()
        return response.get("type") == "pong"

    def _execute_query(self, query: str, tx_id: Optional[int] = None) -> QueryResult:
        payload: dict = {"type": "query", "query": query}
        if tx_id is not None:
            payload["txId"] = tx_id
        self._send(payload)
        response = self._recv()
        if response.get("type") == "error":
            raise QueryError(response.get("message", "Query failed"))
        if response.get("type") != "result":
            raise QueryError(f"Unexpected response type: {response.get('type')}")
        return QueryResult(response.get("rows", []))

    def execute(self, query: str) -> QueryResult:
        """Execute a Cypher query and return all results.

        Args:
            query: Cypher query string.

        Returns:
            :class:`QueryResult` containing all result rows.
        """
        return self._execute_query(query)

    def query(self, query: str) -> QueryResult:
        """Alias for :meth:`execute`."""
        return self.execute(query)

    def query_with_params(self, query: str, params: Dict[str, Any]) -> QueryResult:
        """Execute a query with named parameters.

        Parameters are sent as part of the query string using ``$name`` syntax.
        This method substitutes them on the client side for server versions that
        support the ``$param`` syntax.

        Args:
            query: Cypher query string with ``$name`` placeholders.
            params: Parameter dictionary mapping name to value.

        Returns:
            :class:`QueryResult` containing all result rows.
        """
        # Build parameter fragment and prepend to query
        # e.g. WITH $name AS name, $age AS age
        if params:
            param_parts = []
            for key, value in params.items():
                if isinstance(value, str):
                    param_parts.append(f'"{value}" AS {key}')
                elif isinstance(value, bool):
                    param_parts.append(f'{"true" if value else "false"} AS {key}')
                elif value is None:
                    param_parts.append(f'null AS {key}')
                else:
                    param_parts.append(f'{value} AS {key}')
            prefix = "WITH " + ", ".join(param_parts) + " "
            full_query = prefix + query
        else:
            full_query = query
        return self._execute_query(full_query)

    def stream(self, query: str, chunk_size: int = 100) -> Iterator[Dict[str, str]]:
        """Stream query results row by row.

        Useful for large result sets that don't fit in memory.

        Args:
            query: Cypher query string.
            chunk_size: Number of rows per server chunk.

        Yields:
            Each result row as a dict.
        """
        self._send({"type": "streamQuery", "query": query, "chunkSize": chunk_size})

        # Receive streamStart
        start = self._recv()
        if start.get("type") == "error":
            raise QueryError(start.get("message", "Stream failed"))
        if start.get("type") != "streamStart":
            raise QueryError(f"Expected streamStart, got: {start.get('type')}")

        # Receive chunks until streamEnd
        while True:
            msg = self._recv()
            msg_type = msg.get("type")
            if msg_type == "streamChunk":
                yield from msg.get("rows", [])
            elif msg_type == "streamEnd":
                break
            elif msg_type == "error":
                raise QueryError(msg.get("message", "Stream error"))
            else:
                raise QueryError(f"Unexpected message type: {msg_type}")

    def begin_transaction(self, read_only: bool = False) -> Transaction:
        """Begin a new transaction.

        Args:
            read_only: If ``True``, the transaction is read-only.

        Returns:
            A :class:`Transaction` object.
        """
        self._send({"type": "begin", "readOnly": read_only})
        response = self._recv()
        if response.get("type") == "error":
            raise TransactionError(response.get("message", "Begin transaction failed"))
        if response.get("type") != "transactionBegun":
            raise TransactionError(f"Unexpected response: {response}")
        tx_id = response["txId"]
        return Transaction(self, tx_id)

    @contextmanager
    def transaction(self, read_only: bool = False) -> Generator[Transaction, None, None]:
        """Context manager that automatically commits or rolls back a transaction.

        Example::

            with client.transaction() as tx:
                tx.execute("CREATE (n:Person {name: 'Alice'})")
                tx.execute("CREATE (n:Person {name: 'Bob'})")
        """
        tx = self.begin_transaction(read_only=read_only)
        try:
            yield tx
            if not tx._closed:
                tx.commit()
        except Exception:
            if not tx._closed:
                tx.rollback()
            raise

    def stats(self) -> Dict[str, Any]:
        """Get server statistics.

        Returns:
            Dict with keys: connections, total_queries, nodes, edges.
        """
        self._send({"type": "stats"})
        response = self._recv()
        if response.get("type") != "stats":
            raise MaharitError(f"Unexpected response: {response}")
        return {
            "connections": response.get("connections", 0),
            "total_queries": response.get("total_queries", 0),
            "nodes": response.get("nodes", 0),
            "edges": response.get("edges", 0),
        }

    def close(self) -> None:
        """Close the connection to the server."""
        if self._sock is not None:
            try:
                self._send({"type": "disconnect"})
                self._recv()
            except Exception:
                pass
            finally:
                try:
                    self._sock.close()
                except Exception:
                    pass
                self._sock = None

    def __enter__(self) -> "Client":
        return self

    def __exit__(self, exc_type, exc_val, exc_tb) -> None:
        self.close()

    def __repr__(self) -> str:
        connected = self._sock is not None
        return f"Client({self._host}:{self._port}, connected={connected}, max_retries={self._max_retries})"
