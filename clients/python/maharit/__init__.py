"""MaharitDB Python client library."""

from .client import Client, QueryResult, Transaction
from .async_client import AsyncClient, AsyncQueryResult, AsyncTransaction
from .exceptions import MaharitError, ConnectionError, QueryError, TransactionError
from .models import Edge, Node
from .pool import AsyncConnectionPool, ConnectionPool

__all__ = [
    "Client",
    "QueryResult",
    "Transaction",
    "AsyncClient",
    "AsyncQueryResult",
    "AsyncTransaction",
    "MaharitError",
    "ConnectionError",
    "QueryError",
    "TransactionError",
    "Node",
    "Edge",
    "ConnectionPool",
    "AsyncConnectionPool",
]

__version__ = "0.1.0"
