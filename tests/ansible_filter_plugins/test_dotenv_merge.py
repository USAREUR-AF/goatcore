#!/usr/bin/env python3
"""Tests for dotenv_merge: mgmt-host.yml updates its own keys in /opt/mgmt-host/.env
and keeps every key another role (or an operator) wrote.

Other roles add keys to the same file with lineinfile: the sign-in UI and console
image pins, the Kratos courier URI, the SPIRE join token and database password, the
dashboard and isitup images, and more. When mgmt-host.yml rewrote the whole file,
a routine converge deleted them, and `docker compose up` then recreated those
services on their compose defaults or refused to start them.

    python3 tests/ansible_filter_plugins/test_dotenv_merge.py
"""

import importlib.util
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FILTERS = ROOT / "ops" / "ansible" / "filter_plugins"
spec = importlib.util.spec_from_file_location("dotenv_merge", FILTERS / "dotenv_merge.py")
dm = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dm)

EXISTING = """\
# Rendered by ops/ansible/mgmt-host.yml on 2026-11-01T00:00:00Z
# Do NOT edit by hand; re-run the playbook.
SITE_NAME=old-site
POSTGRES_SUPERUSER_PASSWORD=old-secret
KRATOS_SELFSERVICE_UI_IMAGE=registry.example/ui@sha256:aaa
# written by the spire role
SPIRE_JOIN_TOKEN=tok-123
ID_ADMIN_CONSOLE_IMAGE=registry.example/console@sha256:bbb
TRAEFIK_TLS_ROOT=/etc/goat/tls
"""

RENDERED = """\
# Rendered by ops/ansible/mgmt-host.yml on 2026-11-20T00:00:00Z
# Do NOT edit by hand; re-run the playbook.
SITE_NAME=new-site
POSTGRES_SUPERUSER_PASSWORD=new-secret
TRAEFIK_TLS_ROOT=/etc/goat/tls
SIGNAL_NEGATIVE_CACHE_TTL=30
"""


def keys(text):
    out = {}
    for line in text.splitlines():
        m = re.match(r"^([A-Za-z_][A-Za-z0-9_]*)=(.*)$", line)
        if m:
            out[m.group(1)] = m.group(2)
    return out


class MergeKeepsForeignKeys(unittest.TestCase):
    def test_foreign_keys_survive(self):
        merged = keys(dm.dotenv_merge(EXISTING, RENDERED))
        self.assertEqual(merged["KRATOS_SELFSERVICE_UI_IMAGE"], "registry.example/ui@sha256:aaa")
        self.assertEqual(merged["SPIRE_JOIN_TOKEN"], "tok-123")
        self.assertEqual(merged["ID_ADMIN_CONSOLE_IMAGE"], "registry.example/console@sha256:bbb")

    def test_owned_keys_are_updated(self):
        merged = keys(dm.dotenv_merge(EXISTING, RENDERED))
        self.assertEqual(merged["SITE_NAME"], "new-site")
        self.assertEqual(merged["POSTGRES_SUPERUSER_PASSWORD"], "new-secret")

    def test_new_owned_keys_are_added(self):
        merged = keys(dm.dotenv_merge(EXISTING, RENDERED))
        self.assertEqual(merged["SIGNAL_NEGATIVE_CACHE_TTL"], "30")

    def test_each_key_appears_once(self):
        text = dm.dotenv_merge(EXISTING + "SITE_NAME=dup\n", RENDERED)
        names = [m.group(1) for m in re.finditer(r"^([A-Za-z_][A-Za-z0-9_]*)=", text, re.M)]
        self.assertEqual(len(names), len(set(names)), names)

    def test_order_and_comments_of_existing_file_kept(self):
        text = dm.dotenv_merge(EXISTING, RENDERED)
        self.assertIn("# written by the spire role\nSPIRE_JOIN_TOKEN=tok-123", text)
        self.assertLess(text.index("SITE_NAME="), text.index("SPIRE_JOIN_TOKEN="))

    def test_idempotent(self):
        once = dm.dotenv_merge(EXISTING, RENDERED)
        twice = dm.dotenv_merge(once, RENDERED)
        self.assertEqual(once, twice)

    def test_timestamp_header_does_not_change_the_file(self):
        later = RENDERED.replace("2026-11-20T00:00:00Z", "2026-11-21T00:00:00Z")
        self.assertEqual(dm.dotenv_merge(EXISTING, RENDERED), dm.dotenv_merge(EXISTING, later))

    def test_new_file(self):
        merged = keys(dm.dotenv_merge("", RENDERED))
        self.assertEqual(set(merged), {"SITE_NAME", "POSTGRES_SUPERUSER_PASSWORD",
                                       "TRAEFIK_TLS_ROOT", "SIGNAL_NEGATIVE_CACHE_TTL"})

    def test_values_with_equals_and_quotes_kept_verbatim(self):
        existing = 'POSTGRES_INITDB_ARGS="--locale=C --encoding=UTF8"\nX=a=b\n'
        rendered = 'POSTGRES_INITDB_ARGS="--locale=C --encoding=SQL_ASCII"\n'
        merged = keys(dm.dotenv_merge(existing, rendered))
        self.assertEqual(merged["POSTGRES_INITDB_ARGS"], '"--locale=C --encoding=SQL_ASCII"')
        self.assertEqual(merged["X"], "a=b")


class ReportNamesOnly(unittest.TestCase):
    def test_report_classifies_key_names(self):
        r = dm.dotenv_key_report(EXISTING, RENDERED)
        self.assertEqual(r["added"], ["SIGNAL_NEGATIVE_CACHE_TTL"])
        self.assertEqual(r["changed"], ["POSTGRES_SUPERUSER_PASSWORD", "SITE_NAME"])
        self.assertEqual(r["unchanged"], ["TRAEFIK_TLS_ROOT"])
        self.assertEqual(r["kept"], ["ID_ADMIN_CONSOLE_IMAGE", "KRATOS_SELFSERVICE_UI_IMAGE", "SPIRE_JOIN_TOKEN"])

    def test_report_never_carries_values(self):
        r = dm.dotenv_key_report(EXISTING, RENDERED)
        flat = repr(r)
        for secret in ("old-secret", "new-secret", "tok-123", "sha256:aaa"):
            self.assertNotIn(secret, flat)


class PlaybookMergesInsteadOfOverwriting(unittest.TestCase):
    """mgmt-host.yml must not write .env from a full render any more."""

    def setUp(self):
        self.play = (ROOT / "ops" / "ansible" / "mgmt-host.yml").read_text()

    def test_no_full_render_to_env(self):
        # The old writer: a copy task with dest .env and an inline content render.
        self.assertIsNone(
            re.search(r"ansible\.builtin\.copy:\s*\n\s*dest: \"\{\{ compose_dir \}\}/\.env\"\s*\n\s*mode: \"0600\"\s*\n\s*content: \|", self.play),
            "mgmt-host.yml still overwrites .env with a full render",
        )

    def test_merge_and_report_are_wired(self):
        self.assertIn("dotenv_merge", self.play)
        self.assertIn("dotenv_key_report", self.play)
        self.assertIn("mgmt_env_report_only", self.play)


class RealAnsibleRun(unittest.TestCase):
    """The play's own task shape, run by ansible-playbook on localhost."""

    def test_foreign_keys_survive_a_real_write(self):
        import shutil
        import subprocess
        import tempfile
        if not shutil.which("ansible-playbook"):
            self.skipTest("ansible-playbook not installed")
        with tempfile.TemporaryDirectory() as d:
            env = Path(d) / ".env"
            env.write_text(EXISTING)
            play = Path(d) / "play.yml"
            play.write_text(
                "- hosts: localhost\n"
                "  gather_facts: false\n"
                "  vars:\n"
                f"    compose_dir: {d}\n"
                "    _mgmt_env_rendered: |\n"
                + "".join(f"      {l}\n" for l in RENDERED.splitlines())
                + "  tasks:\n"
                "    - ansible.builtin.slurp: {src: '{{ compose_dir }}/.env'}\n"
                "      register: cur\n"
                "    - ansible.builtin.copy:\n"
                "        dest: '{{ compose_dir }}/.env'\n"
                "        mode: '0600'\n"
                "        content: \"{{ (cur.content | b64decode) | dotenv_merge(_mgmt_env_rendered) }}\"\n"
            )
            envv = dict(__import__("os").environ, ANSIBLE_FILTER_PLUGINS=str(FILTERS),
                        ANSIBLE_LOCALHOST_WARNING="false", ANSIBLE_INVENTORY_UNPARSED_WARNING="false")
            r = subprocess.run(["ansible-playbook", "-i", "localhost,", "-c", "local", str(play)],
                               capture_output=True, text=True, env=envv)
            self.assertEqual(r.returncode, 0, r.stdout[-2000:] + r.stderr[-2000:])
            merged = keys(env.read_text())
            self.assertEqual(merged["SPIRE_JOIN_TOKEN"], "tok-123")
            self.assertEqual(merged["KRATOS_SELFSERVICE_UI_IMAGE"], "registry.example/ui@sha256:aaa")
            self.assertEqual(merged["SITE_NAME"], "new-site")
            self.assertEqual(oct(env.stat().st_mode & 0o777), "0o600")


if __name__ == "__main__":
    unittest.main()
