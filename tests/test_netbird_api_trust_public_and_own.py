#!/usr/bin/env python3
"""The NetBird API tools trust the system store and the goatnet's own CA together.

A goatnet keeps its CA root in the repository whether its management API answers
on that CA (a LAN goatnet) or on a public certificate (an internet goatnet). The
tools verified against the root alone whenever the file existed, so an internet
goatnet's API, on a public certificate, failed verification. The context each
tool builds must hold the system store's authorities and the goatnet's root.
"""
import importlib.util
from pathlib import Path
import ssl
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


def load(name, rel):
    spec = importlib.util.spec_from_file_location(name, ROOT / rel)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def self_signed(dirpath):
    key, cert = Path(dirpath) / "k.pem", Path(dirpath) / "root-cert.pem"
    subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
                    "-keyout", str(key), "-out", str(cert), "-days", "2", "-subj", "/CN=zagros fixture root"],
                   check=True, capture_output=True)
    return cert


class TrustBoth(unittest.TestCase):
    def setUp(self):
        self.system = len(ssl.create_default_context().get_ca_certs())
        if self.system == 0:
            self.skipTest("this machine's default context lists no authority to count")
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        ca_dir = Path(self.tmp.name) / "ops/enrollment/ca/zagros"
        ca_dir.mkdir(parents=True)
        self_signed(ca_dir)

    def check(self, ctx):
        subjects = [dict(x[0] for x in c["subject"]).get("commonName") for c in ctx.get_ca_certs()]
        self.assertIn("zagros fixture root", subjects, "the goatnet's own root is trusted")
        self.assertEqual(len(subjects), self.system + 1, "and the system store's authorities are still trusted")

    def test_policy_posture(self):
        mod = load("posture", "ops/scripts/netbird-policy-posture.py")
        mod.REPO = Path(self.tmp.name)
        self.check(mod._tls_context("zagros"))

    def test_service_identities(self):
        mod = load("identities", "ops/scripts/netbird-service-identities.py")
        mod.REPO = Path(self.tmp.name)
        api = next(getattr(mod, n) for n in dir(mod) if isinstance(getattr(mod, n), type) and "tls" in getattr(getattr(mod, n).__init__, "__code__", type("x", (), {"co_names": ()})).co_names)
        self.check(api("https://example.invalid", "t", site="zagros").tls)


if __name__ == "__main__":
    unittest.main()
