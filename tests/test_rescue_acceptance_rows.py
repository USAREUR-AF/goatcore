#!/usr/bin/env python3
"""The rescue machines' acceptance rows (ADR 1114 section 5, ADR 1131; gates J13
and J15), each run as the runner runs it: through probes.py's cmd_assert, which
reaches the host with ssh and runs the command there as `sudo -n <cmd>`. A
stand-in ssh runs the remote command here, a stand-in sudo runs what it is
given, and stand-ins for ip, ss, netbird and docker play a machine that keeps
the rule and ones that break it. Standard library + PyYAML."""
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
import probes  # noqa: E402 (ops/validation is on the path once the runner is loaded)
with open(os.path.join(REPO, "ops/validation/acceptance-tests.yaml"), encoding="utf-8") as fh:
    DOC = yaml.safe_load(fh)
ROWS = {r["id"]: r for r in (DOC.get("tests") or DOC.get("rows") or next(v for v in DOC.values() if isinstance(v, list)))}


# ssh's last word is the command for the remote shell: run it here. sudo runs
# what it is given, as it does: an assignment is not a program.
SSH = 'for w in "$@"; do last=$w; done\nexec bash -c "$last"\n'
SUDO = '[ "$1" = -n ] && shift\nexec "$@"\n'


def ask(row, **tools):
    """The row's verdict from cmd_assert, with each tool printing what the test says."""
    params = RUNNER.resolve(ROWS[row]["params"], {"breakglass_host": "h", "network_jump_host": "h", "breakglass_oob_interface": OOB})
    with tempfile.TemporaryDirectory() as d:
        for name, script in dict(tools, ssh=SSH, sudo=SUDO).items():
            p = os.path.join(d, name)
            with open(p, "w", encoding="utf-8") as f:
                f.write("#!/bin/sh\n" + script)
            os.chmod(p, 0o700)
        path = os.environ.get("PATH", "")
        os.environ["PATH"] = d + ":/usr/bin:/bin"
        try:
            v = probes.cmd_assert(params)
        finally:
            os.environ["PATH"] = path
    return v.status, v.evidence


class Rescue(unittest.TestCase):
    def test_a_row_whose_command_begins_with_an_assignment_cannot_pass(self):
        # What the runner does to such a command: sudo takes the assignment's
        # output for the program to run. A row must start with a plain command.
        params = {"host": "h", "cmd": "s=$(echo none) && echo \"$s\"", "expect": "none"}
        with tempfile.TemporaryDirectory() as d:
            for name, script in (("ssh", SSH), ("sudo", SUDO)):
                with open(os.path.join(d, name), "w", encoding="utf-8") as f:
                    f.write("#!/bin/sh\n" + script)
                os.chmod(os.path.join(d, name), 0o700)
            path = os.environ.get("PATH", "")
            os.environ["PATH"] = d + ":/usr/bin:/bin"
            try:
                v = probes.cmd_assert(params)
            finally:
                os.environ["PATH"] = path
        self.assertNotEqual(v.status, probes.PASS, v.evidence)
        # Every host row of the manifest, not only these: none may begin with an
        # assignment or a compound that sudo cannot run as one program.
        for row in ROWS.values():
            params = row.get("params") or {}
            cmd = str(params.get("cmd", "")).lstrip()
            if row.get("probe") != "cmd-assert" or not params.get("host") or str(params["host"]) in ("localhost", "127.0.0.1"):
                continue
            first = cmd.split(maxsplit=1)[0] if cmd else ""
            self.assertNotRegex(first, r"^([A-Za-z_][A-Za-z0-9_]*=|[({!]|\[\[?$|if$|for$|while$|case$)", row["id"])

    def holds(self, row, **tools):
        status, evidence = ask(row, **tools)
        self.assertEqual(status, probes.PASS, f"{row}: {evidence}")

    def breaks(self, row, **tools):
        status, evidence = ask(row, **tools)
        self.assertNotEqual(status, probes.PASS, f"{row}: {evidence}")

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
        def udp(*local):
            return "printf '%s'\n" % "".join("udp UNCONN 0 0 %s 0.0.0.0:*\\n" % a.replace("%", "%%") for a in local)
        # The DHCP client's own socket, as networkd holds it once it has a lease,
        # and DHCPv6's: a client, not a service.
        self.holds(row, ip=ip, ss=udp("0.0.0.0%wwan0:68", "198.51.100.7%wwan0:68", "[fe80::7]%wwan0:546"))
        # Anything else on those ports, or any other UDP service, is counted.
        self.breaks(row, ip=ip, ss=ss("0.0.0.0%wwan0:68").replace("%w", "%%w"))
        self.breaks(row, ip=ip, ss=udp("0.0.0.0%wwan0:67"))
        self.breaks(row, ip=ip, ss=udp("198.51.100.7:53"))
        self.breaks(row, ip=ip, ss=udp("198.51.100.7:5468"))
        self.holds(row, ip=ip, ss=ss("198.51.100.70:22"))
        # expect is a substring: ten services must not read as "0".
        self.breaks(row, ip=ip, ss=ss(*["198.51.100.7:%d" % p for p in range(8080, 8090)]))

    def test_the_jump_box(self):
        self.holds("network-jump-node-is-on-the-mesh", netbird="echo 'Management: Connected'\n")
        self.breaks("network-jump-node-is-on-the-mesh", netbird="echo 'Management: Disconnected'\n")
        self.breaks("network-jump-node-is-on-the-mesh")
        row = "network-jump-accepts-only-ssh"

        def ss(*local):
            # Each listener as ss -Hltnp prints it; "addr users:((...))" adds the process column.
            def line(a):
                addr, _, users = a.partition(" ")
                return "LISTEN 0 128 %s 0.0.0.0:*%s\\n" % (addr, " " + users if users else "")
            return "printf '%s'\n" % "".join(line(a) for a in local)

        nb = ' users:(("netbird",pid=812,fd=41))'
        other = ' users:(("dnsmasq",pid=990,fd=5))'
        both = ' users:(("netbird",pid=812,fd=41),("dnsmasq",pid=990,fd=5))'
        # The machine's mesh address, as ip reads it from wt0; never the test host's own.
        ip = "printf '9: wt0    inet 100.127.0.5/16 brd 100.127.255.255 scope global wt0\\n'\n"
        self.holds(row, ip=ip, ss=ss("0.0.0.0:22", "[::]:22", "127.0.0.1:9090", "[::1]:631", "127.0.0.53:53"))
        self.breaks(row, ip=ip, ss=ss("0.0.0.0:22", "0.0.0.0:8080"))
        self.breaks(row, ip=ip, ss=ss("[::]:2222"))
        self.breaks(row, ip=ip, ss=ss("192.0.2.9:9122"))
        self.breaks(row, ip=ip, ss=ss("0.0.0.0:22", *["0.0.0.0:%d" % p for p in range(8080, 8090)]))
        # NetBird's own resolver on the machine's own mesh address is the mesh's.
        self.holds(row, ip=ip, ss=ss("0.0.0.0:22", "100.127.0.5:53" + nb))
        # Another process on the mesh address's port 53 is counted, alone or beside netbird.
        self.breaks(row, ip=ip, ss=ss("0.0.0.0:22", "100.127.0.5:53" + other))
        self.breaks(row, ip=ip, ss=ss("0.0.0.0:22", "100.127.0.5:53" + both))
        # A listener ss names no process for (no root, or the column missing) is counted.
        self.breaks(row, ip=ip, ss=ss("0.0.0.0:22", "100.127.0.5:53"))
        # netbird on any other address or port is counted.
        self.breaks(row, ip=ip, ss=ss("0.0.0.0:22", "0.0.0.0:53" + nb))
        self.breaks(row, ip=ip, ss=ss("0.0.0.0:22", "192.0.2.9:53" + nb))
        self.breaks(row, ip=ip, ss=ss("0.0.0.0:22", "100.127.0.6:53" + nb))
        self.breaks(row, ip=ip, ss=ss("0.0.0.0:22", "100.127.0.5:5353" + nb))
        self.breaks(row, ip=ip, ss=ss("0.0.0.0:22", "100.127.0.5:530" + nb))
        # With no mesh address there is no resolver of the mesh's to skip.
        self.breaks(row, ip="true\n", ss=ss("0.0.0.0:22", "100.127.0.5:53" + nb))
        # A name that only begins with netbird is not netbird.
        self.breaks(row, ip=ip, ss=ss("0.0.0.0:22", '100.127.0.5:53 users:(("netbirdx",pid=7,fd=3))'))
        row = "network-jump-connector-runs"
        self.holds(row, docker="[ \"$3\" = '{{.State.Running}}' ] && echo true\n")
        self.breaks(row, docker="echo false\n")
        self.breaks(row, docker="exit 1\n")

    def test_the_clock_step_is_armed_only_with_both_units_enabled(self):
        row = "breakglass-clock-recovery-armed"

        def states(*s):
            return "printf '%s'\n" % "".join("%s\\n" % x for x in s)
        self.holds(row, systemctl=states("enabled", "enabled"))
        # "disabled" holds "enabled": a substring expect read it as armed.
        self.breaks(row, systemctl=states("disabled", "enabled"))
        self.breaks(row, systemctl=states("enabled", "not-found"))
        self.breaks(row, systemctl=states("enabled"))
        self.breaks(row, systemctl="exit 1\n")

    def test_a_tool_that_fails_is_never_a_pass(self):
        fails = "echo nothing >&2; exit 1\n"
        self.breaks("breakglass-oob-is-never-the-default-route", ip=fails)
        self.breaks("breakglass-oob-binds-no-service", ip=fails, ss="true\n")
        self.breaks("breakglass-oob-binds-no-service", ip="echo '3: wwan0 inet 198.51.100.7/24'\n", ss=fails)
        self.breaks("network-jump-accepts-only-ssh", ip="true\n", ss=fails)
        self.breaks("network-jump-node-is-on-the-mesh", netbird=fails)
        self.breaks("network-jump-connector-runs", docker=fails)

    def test_the_jump_box_row_says_it_counts_tcp_listeners_only(self):
        row = ROWS["network-jump-accepts-only-ssh"]
        self.assertIn("ss -Hltn", row["params"]["cmd"])
        self.assertNotIn("-u", row["params"]["cmd"].split("ss ")[1].split()[0])
        self.assertIn("A TCP service other than SSH", row["caught_would_have"])
        self.assertIn("UDP listeners are not counted", row["caught_would_have"])

    def test_the_rows_ask_only_where_the_machine_is_declared(self):
        for row, when in (("breakglass-clock-recovery-armed", "breakglass_clock_step_armed"),
                          ("breakglass-oob-is-never-the-default-route", "breakglass_has_oob"),
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

    def test_the_clock_row_follows_the_role(self):
        # Off by the role's default: the HTTP time source is off.
        self.assertFalse(self.values(dict(self.BASE))["breakglass_clock_step_armed"])
        on = {**self.BASE, "host_vars/bg-1.yml": "breakglass_http_time_enabled: true\n"}
        self.assertTrue(self.values(on)["breakglass_clock_step_armed"])
        self.assertTrue(self.values({**self.BASE, "group_vars/zagros_breakglass_hosts/vars.yml": "breakglass_http_time_enabled: \"yes\"\n"})["breakglass_clock_step_armed"])
        # The step itself turned off, on the node over its group.
        off = {**on, "host_vars/bg-1/clock.yml": "breakglass_clock_step_enabled: false\n"}
        self.assertFalse(self.values(off)["breakglass_clock_step_armed"])
        # No break-glass node: nothing to arm.
        none = {k: v for k, v in on.items() if "breakglass-nodes" not in k}
        self.assertFalse(self.values(none)["breakglass_clock_step_armed"])

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
