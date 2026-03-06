"""MaharitDB exceptions."""


class MaharitError(Exception):
    """Base exception for all MaharitDB errors."""


class ConnectionError(MaharitError):
    """Raised when a connection to the server cannot be established or is lost."""


class QueryError(MaharitError):
    """Raised when the server returns an error response to a query."""


class TransactionError(MaharitError):
    """Raised when a transaction operation fails."""
