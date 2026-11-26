#!/usr/bin/env python3
"""netbird-policy-posture.py and netbird-service-identities.py verify the mgmt API
against the goatnet's own root CA when it is in the repo, unless NETBIRD_MGMT_CA=public
(an internet goatnet with its own CA whose mgmt API has a public ACME certificate),
which keeps the system store. Verification is on in every case.

Builds a throwaway root CA under a temp REPO; no network.
Run: python3 -m unittest tests/test_netbird_mgmt_ca.py -v
"""
import importlib.util
import os
import ssl
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
SITE = "testsite"


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), ROOT / "ops/scripts" / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class MgmtCa(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name)
        ca_dir = self.repo / "ops/enrollment/ca" / SITE
        ca_dir.mkdir(parents=True)
        self.root = ca_dir / "root-cert.pem"
        subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
                        "-nodes", "-keyout", str(ca_dir / "k.pem"), "-out", str(self.root), "-days", "1",
                        "-subj", "/CN=test root"], check=True, capture_output=True)

    def tearDown(self):
        self.tmp.cleanup()

    def cafiles(self, make, env):
        environ = {k: v for k, v in os.environ.items() if k != "NETBIRD_MGMT_CA"}
        environ.update(env)
        with mock.patch.dict(os.environ, environ, clear=True), \
             mock.patch.object(ssl, "create_default_context", wraps=ssl.create_default_context) as cdc:
            ctx = make()
        self.assertEqual(ctx.verify_mode, ssl.CERT_REQUIRED, "verification must stay on")
        return [c.kwargs.get("cafile") for c in cdc.call_args_list]

    def posture(self):
        mod = load("netbird-policy-posture")
        mod.REPO = self.repo
        return lambda: mod._tls_context(SITE)

    def identities(self):
        mod = load("netbird-service-identities")
        mod.REPO = self.repo
        return lambda: mod.NetBird("https://mgmt.example", "t", SITE).tls

    def test_default_pins_the_site_root(self):
        for make in (self.posture(), self.identities()):
            self.assertEqual(self.cafiles(make, {}), [str(self.root)])

    def test_public_uses_the_system_store(self):
        for make in (self.posture(), self.identities()):
            self.assertEqual(self.cafiles(make, {"NETBIRD_MGMT_CA": "public"}), [None])


if __name__ == "__main__":
    unittest.main()
