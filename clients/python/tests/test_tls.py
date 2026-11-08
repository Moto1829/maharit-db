"""Tests for the TLS option handling shared by the clients."""

import os
import ssl
import tempfile
import unittest

from maharit._tls import make_ssl_context

# Self-signed localhost certificate (CA:FALSE), only used as a CA file here.
_CERT = """-----BEGIN CERTIFICATE-----
MIIBcjCCARqgAwIBAgIUOj78s7mMRH8iVIfg46EKD96CZKYwCgYIKoZIzj0EAwIw
FDESMBAGA1UEAwwJbG9jYWxob3N0MCAXDTI2MTEwODA2NTE1MVoYDzIxMjYxMDE1
MDY1MTUxWjAUMRIwEAYDVQQDDAlsb2NhbGhvc3QwWTATBgcqhkjOPQIBBggqhkjO
PQMBBwNCAATKMCUihiKFQLw4KLkmCOgC7wGn2MDnWvpTIOQyoR/9mUq7Ny/SZPlb
qrZfVAtpTlAdceW2nU3t7kIZQB3SueRro0gwRjAdBgNVHQ4EFgQUM6B5DLeRP506
1vHcwQaTje6SevQwDwYDVR0TAQH/BAUwAwEB/zAUBgNVHREEDTALgglsb2NhbGhv
c3QwCgYIKoZIzj0EAwIDRgAwQwIfWg44Ca+fabqyVX5tE/lSTx0XSehqVyCAn6ye
WFbCOAIgKzo0xyfWqsiWwnsRcOTvxg//VGRN2ALQYz+d5qS8NuE=
-----END CERTIFICATE-----
"""


class MakeSslContextTest(unittest.TestCase):
    def test_plain_tcp_by_default(self):
        self.assertIsNone(make_ssl_context())
        self.assertIsNone(make_ssl_context(False))

    def test_true_verifies_certificates(self):
        ctx = make_ssl_context(True)
        self.assertIsInstance(ctx, ssl.SSLContext)
        self.assertEqual(ctx.verify_mode, ssl.CERT_REQUIRED)
        self.assertTrue(ctx.check_hostname)

    def test_ca_file_enables_tls_and_trusts_it(self):
        with tempfile.NamedTemporaryFile("w", suffix=".pem", delete=False) as f:
            f.write(_CERT)
        try:
            ctx = make_ssl_context(ca_file=f.name)
            self.assertIsNotNone(ctx)
            self.assertEqual(ctx.verify_mode, ssl.CERT_REQUIRED)
            self.assertEqual(ctx.cert_store_stats()["x509"], 1)
        finally:
            os.unlink(f.name)

    def test_context_is_used_as_is(self):
        ctx = ssl.create_default_context()
        self.assertIs(make_ssl_context(ctx), ctx)


if __name__ == "__main__":
    unittest.main()
