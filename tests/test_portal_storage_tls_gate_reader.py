#!/usr/bin/env python3
"""The portal's storage-TLS gate counts an audit reader only when it has an endpoint.

portal.env passes PORTAL_AUDIT_S3_ENDPOINT, and with it empty the portal opens
no storage client (cmd/portal/main.go checks storage TLS only for a non-empty
endpoint). The gate still refused an HTTPS-only image whenever the reader's
secret was set, even with portal_audit_s3_endpoint set empty: a refusal for a
storage read that never happens. The gate's own assert runs here under
ansible-playbook."""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
GATE = ROOT / "ops/ansible/roles/goat-zenoh-portal/tasks/storage-tls-gate.yml"
NAME = "portal | storage-TLS gate — an HTTPS-only audit reader needs the storage TLS variables"
SECRET = "ZAGROS_PORTAL_AUDIT_S3_SECRET_KEY"


class _Loader(yaml.SafeLoader):
    """SafeLoader that reads Ansible's !unsafe tag as a plain string."""


_Loader.add_constructor("!unsafe", lambda loader, node: loader.construct_scalar(node))


def gate_task():
    return next(t for t in yaml.load(GATE.read_text(), Loader=_Loader) if t.get("name") == NAME)


class ReaderNeedsAnEndpoint(unittest.TestCase):

    def gate(self, extra_vars, secret="s3cret"):
        if not shutil.which("ansible-playbook"):
            self.skipTest("ansible-playbook is required")
        d = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, d)
        task = gate_task()
        labels = '{"io.goat.zenoh-portal.storage-tls": "required"}'
        play = [{"hosts": "localhost", "gather_facts": False, "tasks": [task], "vars": dict({
            "site_id": "zagros", "portal_image": "portal@sha256:" + "0" * 64,
            "portal_tls_gate_labels": {"rc": 0, "stdout": labels},
        }, **extra_vars)}]
        (d / "play.yml").write_text(yaml.safe_dump(play))
        env = dict(os.environ, ANSIBLE_NOCOLOR="1", ANSIBLE_LOCALHOST_WARNING="False")
        env.pop("ANSIBLE_CONFIG", None)
        env.pop(SECRET, None)
        if secret:
            env[SECRET] = secret
        return subprocess.run(["ansible-playbook", "-i", "localhost,", "-c", "local", str(d / "play.yml")],
                              cwd=d, env=env, capture_output=True, text=True, timeout=120)

    def test_a_reader_with_an_http_endpoint_is_refused(self):
        r = self.gate({"portal_audit_s3_endpoint": "http://storage.zagros.example:9000"})
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("reads audit storage over verified HTTPS only", r.stdout)

    def test_an_undefined_endpoint_means_the_role_default_and_is_refused(self):
        self.assertNotEqual(self.gate({}).returncode, 0)

    def test_an_endpoint_set_empty_means_no_reader(self):
        r = self.gate({"portal_audit_s3_endpoint": ""})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_tls_on_with_no_storage_endpoint_means_no_reader(self):
        r = self.gate({"minio_tls_enabled": True, "minio_s3_endpoint": ""})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_tls_on_with_an_https_endpoint_passes(self):
        r = self.gate({"minio_tls_enabled": True, "minio_s3_endpoint": "https://storage.zagros.example:9000"})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_no_secret_means_no_reader(self):
        r = self.gate({"portal_audit_s3_endpoint": "http://storage.zagros.example:9000"}, secret="")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)


if __name__ == "__main__":
    unittest.main()
