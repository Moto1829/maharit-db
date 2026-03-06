"""MaharitDB Python client library."""

from .client import Client, QueryResult, Transaction
from .async_client import AsyncClient, AsyncQueryResult, AsyncTransaction
from .exceptions import MaharitError, ConnectionError, QueryError, TransactionError

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
]

__version__ = "0.1.0"
