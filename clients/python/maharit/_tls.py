"""TLS settings shared by the synchronous and asynchronous clients."""

import ssl as _ssl
from typing import Optional, Union

#: Accepted values for the ``ssl`` argument of the clients and pools.
SslOption = Union[bool, _ssl.SSLContext, None]


def make_ssl_context(
    ssl: SslOption = None, ca_file: Optional[str] = None
) -> Optional[_ssl.SSLContext]:
    """Build the SSL context to connect with, or ``None`` for plain TCP.

    Args:
        ssl: ``True`` verifies the server certificate against the system CA
            store (or ``ca_file``); an :class:`ssl.SSLContext` is used as is;
            ``None`` / ``False`` connect without TLS unless ``ca_file`` is set.
        ca_file: PEM file with the CA certificate (or the server's
            self-signed certificate) to trust. Setting it enables TLS.
    """
    if isinstance(ssl, _ssl.SSLContext):
        if ca_file is not None:
            ssl.load_verify_locations(cafile=ca_file)
        return ssl
    if not ssl and ca_file is None:
        return None
    return _ssl.create_default_context(cafile=ca_file)
