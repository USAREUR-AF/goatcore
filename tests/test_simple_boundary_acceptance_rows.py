#!/usr/bin/env python3
"""The simple boundary's acceptance rows (ADR 1253; gate J10), each run as written
with stand-ins for netbird, systemctl, sudo, ip and curl, on a machine that keeps
the rule and on ones that break it; and generate-values, which turns the rows on
for a site whose inventory holds a simple-boundary machine. Standard library +
PyYAML."""
import importlib.util
import json
import os
import runpy
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

import yaml

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
STATUS = "/run/simple-bn/status.json"


def load_runner():
    spec = importlib.util.spec_from_file_location("run_acceptance", os.path.join(REPO, "ops/validation/run-acceptance.py"))
    m = importlib.util.module_from_spec(spec)
    sys.path.insert(0, os.path.join(REPO, "ops/validation"))
    spec.loader.exec_module(m)
    return m


RUNNER = load_runner()
with open(os.path.join(REPO, "ops/validation/acceptance-tests.yaml"), encoding="utf-8") as fh:
    ROWS = {r["id"]: r for r in yaml.safe_load(fh)["tests"]}
IDS = ["simple-boundary-is-on-the-mesh", "simple-boundary-services-run", "simple-boundary-sender-is-sending", "simple-boundary-monitor-is-mesh-only"]


def ask(row, status=None, **tools):
    """The row's command's stdout, with each tool printing what the test says. The
    sender's status file is read from a file the test writes, not from /run."""
    params = RUNNER.resolve(ROWS[row]["params"], {"simple_boundary_host": "h"})
    with tempfile.TemporaryDirectory() as d:
        tools.setdefault("sudo", 'exec "$@"\n')
        for name, script in tools.items():
            p = os.path.join(d, name)
            with open(p, "w", encoding="utf-8") as f:
                f.write("#!/bin/sh\n" + script)
            os.chmod(p, 0o700)
        cmd = params["cmd"]
        if status is not None:
            path = os.path.join(d, "status.json")
            with open(path, "w", encoding="utf-8") as f:
                f.write(status)
            cmd = cmd.replace(STATUS, path)
        out = subprocess.run(["bash", "-c", cmd], capture_output=True, text=True,
                             env={"PATH": d + ":/usr/bin:/bin"}, check=False)
    return out.stdout.strip(), str(params["expect"])


def status(*states, age=0):
    return json.dumps({"now": time.time() - age, "feeds": [{"feed": "radar-%d" % i, "state": s} for i, s in enumerate(states)]})


class SimpleBoundary(unittest.TestCase):
    def holds(self, row, **kw):
        got, want = ask(row, **kw)
        self.assertEqual(got, want, row)

    def breaks(self, row, **kw):
        got, want = ask(row, **kw)
        self.assertNotEqual(got, want, row)

    def test_on_the_mesh(self):
        row = "simple-boundary-is-on-the-mesh"
        self.holds(row, netbird="echo 'Management: Connected'\n")
        self.breaks(row, netbird="echo 'Management: Disconnected'\n")

    def test_both_services_run(self):
        row = "simple-boundary-services-run"
        self.holds(row, systemctl="printf 'active\\nactive\\n'\n")
        self.breaks(row, systemctl="printf 'active\\ninactive\\n'; exit 3\n")
        self.breaks(row, systemctl="printf 'failed\\nactive\\n'; exit 3\n")

    def test_the_sender_is_sending(self):
        row = "simple-boundary-sender-is-sending"
        self.holds(row, status=status("sending", "sending"))
        self.holds(row, status=status("sending", "receiver-refusing"))
        self.holds(row, status=status("holding"))
        self.breaks(row, status=status("sending", "send-error"))
        self.breaks(row, status=status("waiting-for-input"))
        self.breaks(row, status=status("sending", age=120))
        self.breaks(row, status=status())
        self.breaks(row, status="not json")

    def test_the_monitor_answers_on_the_mesh_address_only(self):
        row = "simple-boundary-monitor-is-mesh-only"
        ip = "echo '7: wt0    inet 100.127.0.9/32 scope global wt0'\n"
        mesh_only = 'case "$*" in *100.127.0.9:8740*) echo ok ;; *) exit 7 ;; esac\n'
        self.holds(row, ip=ip, curl=mesh_only)
        self.breaks(row, ip=ip, curl='case "$*" in *100.127.0.9:8740*) echo ok ;; *) exit 0 ;; esac\n')
        self.breaks(row, ip=ip, curl="exit 7\n")
        self.breaks(row, ip="true\n", curl=mesh_only)

    def test_a_tool_that_fails_is_never_a_pass(self):
        fails = "echo nothing >&2; exit 1\n"
        self.breaks("simple-boundary-is-on-the-mesh", netbird=fails)
        self.breaks("simple-boundary-services-run", systemctl=fails)
        self.breaks("simple-boundary-sender-is-sending", sudo=fails, status=status("sending"))
        self.breaks("simple-boundary-monitor-is-mesh-only", ip=fails, curl="echo ok\n")

    def test_the_rows_ask_only_where_the_machine_is(self):
        for row in IDS:
            self.assertEqual(ROWS[row]["applies_when"], "has_simple_boundary")
            self.assertEqual(ROWS[row]["params"]["host"], "{{ simple_boundary_host }}")


class Values(unittest.TestCase):
    """generate-values finds the machine in its type's group and turns the rows on."""

    def values(self, inventory):
        gen = runpy.run_path(os.path.join(REPO, "ops/validation/generate-values.py"))
        with tempfile.TemporaryDirectory() as d:
            ansible = Path(d, "ops/ansible")
            for rel, text in {"group_vars/zagros_hosts/vars.yml": "site_id: zagros\n", "inventory/mgmt-hosts.ini": inventory}.items():
                p = ansible / rel
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_text(text)
            inv = ansible / "inventory/mgmt-hosts.ini"
            gen["generate"].__globals__["INV"] = inv
            return gen["generate"]("zagros", gv_dir=ansible / "group_vars/zagros_hosts", inv_path=inv)

    def test_a_site_with_a_simple_boundary(self):
        v = self.values("[zagros_hosts]\nmgmt-zagros-01-a ansible_host=192.0.2.10\n[zagros_simple_boundary_hosts]\nsbn-1 ansible_host=192.0.2.30\n")
        self.assertEqual(v["has_simple_boundary"], "true")
        self.assertIn("192.0.2.30", v["simple_boundary_host"])
        self.assertTrue(RUNNER.truthy_applies("has_simple_boundary", v))

    def test_a_site_with_none_reads_not_applicable(self):
        v = self.values("[zagros_hosts]\nmgmt-zagros-01-a ansible_host=192.0.2.10\n")
        self.assertEqual(v["has_simple_boundary"], "false")
        self.assertNotIn("simple_boundary_host", v)
        self.assertFalse(RUNNER.truthy_applies("has_simple_boundary", v))


if __name__ == "__main__":
    unittest.main()
