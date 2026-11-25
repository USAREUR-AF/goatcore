#!/usr/bin/env python3
"""The rescue machines' acceptance rows (ADR 1114 section 5, ADR 1131; gates J13
and J15), each run as written with stand-ins for ip, ss, netbird and docker, on a
machine that keeps the rule and on ones that break it. Standard library + PyYAML."""
import importlib.util
import os
import subprocess
import tempfile
import unittest

import yaml

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OOB = "wwan0"


def load_runner():
    spec = importlib.util.spec_from_file_location("run_acceptance", os.path.join(REPO, "ops/validation/run-acceptance.py"))
    m = importlib.util.module_from_spec(spec)
    import sys
    sys.path.insert(0, os.path.join(REPO, "ops/validation"))
    spec.loader.exec_module(m)
    return m


RUNNER = load_runner()
with open(os.path.join(REPO, "ops/validation/acceptance-tests.yaml"), encoding="utf-8") as fh:
    DOC = yaml.safe_load(fh)
ROWS = {r["id"]: r for r in (DOC.get("tests") or DOC.get("rows") or next(v for v in DOC.values() if isinstance(v, list)))}


def ask(row, **tools):
    """The row's command's stdout, with each tool printing what the test says."""
    params = RUNNER.resolve(ROWS[row]["params"], {"breakglass_host": "h", "network_jump_host": "h", "breakglass_oob_interface": OOB})
    with tempfile.TemporaryDirectory() as d:
        for name, script in tools.items():
            p = os.path.join(d, name)
            with open(p, "w", encoding="utf-8") as f:
                f.write("#!/bin/sh\n" + script)
            os.chmod(p, 0o700)
        out = subprocess.run(["bash", "-c", params["cmd"]], capture_output=True, text=True,
                             env={"PATH": d + ":/usr/bin:/bin"}, check=False)
    return out.stdout.strip(), str(params["expect"])


class Rescue(unittest.TestCase):
    def holds(self, row, **tools):
        got, want = ask(row, **tools)
        self.assertEqual(got, want, row)

    def breaks(self, row, **tools):
        got, want = ask(row, **tools)
        self.assertNotEqual(got, want, row)

    def test_the_out_of_band_interface_is_never_the_default_route(self):
        row = "breakglass-oob-is-never-the-default-route"

        def routes(v4, v6=""):
            return "case \"$1\" in link) exit 0 ;; -4) printf '%s' ;; -6) printf '%s' ;; esac\n" % (v4, v6)
        eth, oob = "default via 192.0.2.1 dev eth0 proto dhcp metric 100\\n", "default via 198.51.100.1 dev wwan0 proto dhcp metric %s\\n"
        self.holds(row, ip=routes(eth + oob % 1000))
        self.holds(row, ip=routes(eth))
        self.holds(row, ip=routes(eth + oob % 1000, "default via fe80::1 dev eth0 metric 1024\\n").replace("metric 1000", "metric 1025"))
        self.breaks(row, ip=routes(eth + oob % 100))
        self.breaks(row, ip=routes(eth + oob % 50))
        self.breaks(row, ip=routes(eth + "default via 198.51.100.1 dev wwan0\\n"))
        self.breaks(row, ip=routes(oob % 1000))
        self.breaks(row, ip=routes(eth + oob % 1000, "default via fe80::1 dev wwan0 metric 50\\n"))
        # One default of another interface above the reach-in path is enough.
        self.breaks(row, ip=routes(eth + oob % 1000 + "default via 203.0.113.1 dev eth1 metric 2000\\n"))

    def test_nothing_is_bound_to_the_out_of_band_interface(self):
        row = "breakglass-oob-binds-no-service"
        ip = "printf '3: wwan0 inet 198.51.100.7/24 scope global wwan0\\n3: wwan0 inet6 2001:db8::7/64 scope global\\n'\n"

        def ss(*local):
            return "printf '%s'\n" % "".join("tcp LISTEN 0 128 %s 0.0.0.0:*\\n" % a for a in local)
        self.holds(row, ip=ip, ss=ss("127.0.0.1:8080", "192.0.2.9:22", "0.0.0.0:22", "[::]:22"))
        self.holds(row, ip=ip, ss="true\n")
        self.breaks(row, ip=ip, ss=ss("198.51.100.7:8080"))
        self.breaks(row, ip=ip, ss=ss("[2001:db8::7]:8080"))
        self.breaks(row, ip=ip, ss=ss("0.0.0.0%wwan0:68").replace("%w", "%%w"))
        self.holds(row, ip=ip, ss=ss("198.51.100.70:22"))

    def test_the_jump_box(self):
        self.holds("network-jump-node-is-on-the-mesh", netbird="echo 'Management: Connected'\n")
        self.breaks("network-jump-node-is-on-the-mesh", netbird="echo 'Management: Disconnected'\n")
        self.breaks("network-jump-node-is-on-the-mesh")
        row = "network-jump-accepts-only-ssh"

        def ss(*local):
            return "printf '%s'\n" % "".join("LISTEN 0 128 %s 0.0.0.0:*\\n" % a for a in local)
        self.holds(row, ss=ss("0.0.0.0:22", "[::]:22", "127.0.0.1:9090", "[::1]:631", "127.0.0.53:53"))
        self.breaks(row, ss=ss("0.0.0.0:22", "0.0.0.0:8080"))
        self.breaks(row, ss=ss("[::]:2222"))
        self.breaks(row, ss=ss("192.0.2.9:9122"))
        row = "network-jump-connector-runs"
        self.holds(row, docker="[ \"$3\" = '{{.State.Running}}' ] && echo true\n")
        self.breaks(row, docker="echo false\n")
        self.breaks(row, docker="exit 1\n")

    def test_a_tool_that_fails_is_never_a_pass(self):
        fails = "echo nothing >&2; exit 1\n"
        self.breaks("breakglass-oob-is-never-the-default-route", ip=fails)
        self.breaks("breakglass-oob-binds-no-service", ip=fails, ss="true\n")
        self.breaks("breakglass-oob-binds-no-service", ip="echo '3: wwan0 inet 198.51.100.7/24'\n", ss=fails)
        self.breaks("network-jump-accepts-only-ssh", ss=fails)
        self.breaks("network-jump-node-is-on-the-mesh", netbird=fails)
        self.breaks("network-jump-connector-runs", docker=fails)

    def test_the_rows_ask_only_where_the_machine_is_declared(self):
        for row, when in (("breakglass-oob-is-never-the-default-route", "breakglass_has_oob"),
                          ("breakglass-oob-binds-no-service", "breakglass_has_oob"),
                          ("network-jump-node-is-on-the-mesh", "has_network_jump_node"),
                          ("network-jump-accepts-only-ssh", "has_network_jump_node"),
                          ("network-jump-connector-runs", "has_network_jump_node")):
            self.assertEqual(ROWS[row]["applies_when"], when)


class OutOfBandValue(unittest.TestCase):
    """generate-values reads the out-of-band interface where Ansible would for
    the breakglass node: a per-node host_vars value turns the rows on."""

    def values(self, files):
        import runpy
        from pathlib import Path
        gen = runpy.run_path(os.path.join(REPO, "ops/validation/generate-values.py"))
        with tempfile.TemporaryDirectory() as d:
            ansible = Path(d, "ops/ansible")
            for rel, text in files.items():
                p = ansible / rel
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_text(text)
            inv = ansible / "inventory/mgmt-hosts.ini"
            # run_path returns a copy of the module's globals: set the one its
            # functions read.
            gen["generate"].__globals__["INV"] = inv
            return gen["generate"]("zagros", gv_dir=ansible / "group_vars/zagros_hosts", inv_path=inv)

    BASE = {
        "group_vars/zagros_hosts/vars.yml": "site_id: zagros\n",
        "inventory/mgmt-hosts.ini": "[zagros_hosts]\nmgmt-zagros-01-a ansible_host=192.0.2.10\n",
        "inventory/zagros-breakglass-nodes.ini": "[zagros_breakglass_hosts]\nbg-1 ansible_host=192.0.2.20 ansible_user=goat\n",
    }

    def test_a_per_node_interface_in_host_vars_turns_the_rows_on(self):
        for where in ("host_vars/bg-1.yml", "host_vars/bg-1/oob.yml"):
            v = self.values({**self.BASE, where: f"breakglass_oob_interface: {OOB}\n"})
            self.assertEqual(v.get("breakglass_host"), "goat@192.0.2.20")
            self.assertTrue(v["breakglass_has_oob"], where)
            self.assertEqual(v["breakglass_oob_interface"], OOB, where)

    def test_the_node_overrides_its_group_and_a_group_file_counts(self):
        v = self.values({**self.BASE, "group_vars/zagros_breakglass_hosts.yml": "breakglass_oob_interface: eth9\n"})
        self.assertEqual(v["breakglass_oob_interface"], "eth9")
        v = self.values({**self.BASE, "group_vars/zagros_breakglass_hosts/vars.yml": "breakglass_oob_interface: eth9\n",
                         "host_vars/bg-1.yml": f"breakglass_oob_interface: {OOB}\n"})
        self.assertEqual(v["breakglass_oob_interface"], OOB)
        v = self.values(dict(self.BASE))
        self.assertFalse(v["breakglass_has_oob"])


if __name__ == "__main__":
    unittest.main()
