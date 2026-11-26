"""A host that serves its site's LAN zone resolves that zone itself.

The lan-coredns role answers the zone on the host's LAN address, but nothing sent
the zone to that server for the host itself: its resolver asked the uplink's
server, which does not hold the zone. On a goatlite's box, `goat status` then
failed its portal leg (the acceptance row goat-status-chain) because the box
could not resolve portal.<name>.<zone>. A systemd-resolved drop-in that routes
only the zone to the bind address fixed it on a VM founder.

Each case runs the role's tasks/host-zone-route.yml on localhost, with the
drop-in directory in the test's directory and stand-ins first on PATH for
systemctl (records each call) and getent (answers the smoke-test name only while
the drop-in routes the zone, as the host's resolver would). Become is off;
nothing outside the test's directory is written.

Checked:
  - on: the drop-in routes only the zone to the bind address, systemd-resolved
    is restarted once, and the host is checked to resolve the smoke-test name;
  - on, a second run: nothing changes and nothing restarts;
  - on, a port other than 53: the route names the port;
  - on, the host still does not resolve the name: the run fails and says why;
  - off: an existing drop-in is removed and systemd-resolved restarted; with
    none, nothing happens;
  - a goatlite's render turns the route on.

Run: python3 -m pytest tests/test_lan_coredns_host_zone_route.py
"""
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
TASKS = ROOT / "ops/ansible/roles/lan-coredns/tasks/host-zone-route.yml"
DROPIN = "50-goat-lan-zone.conf"
ZONE, BIND, NAME = "zagros.lan", "192.0.2.50", "mgmt-zagros-01.zagros.lan"


def require_ansible_playbook(case):
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


class HostZoneRoute(unittest.TestCase):

    def setUp(self):
        require_ansible_playbook(self)
        self.d = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.d)
        self.dropins = self.d / "resolved.conf.d"
        self.calls = self.d / "systemctl.calls"
        stubs = self.d / "bin"
        stubs.mkdir()
        (stubs / "systemctl").write_text(f'#!/bin/sh\necho "$*" >> "{self.calls}"\n')
        # Answers the name only while a drop-in routes the zone to the bind address.
        (stubs / "getent").write_text(textwrap.dedent(f"""\
            #!/bin/sh
            f="{self.dropins}/{DROPIN}"
            [ "$NB_GETENT_BROKEN" = 1 ] && exit 2
            if [ -f "$f" ] && grep -q "^DNS={BIND}" "$f" && grep -q "^Domains=~{ZONE}$" "$f"; then
              echo "{BIND}       $2"; exit 0
            fi
            exit 2
            """))
        for p in stubs.iterdir():
            p.chmod(0o755)
        self.path = f"{stubs}:{os.environ.get('PATH', '')}"

    def run_tasks(self, enabled, port=53, broken=False):
        play = self.d / "play.yml"
        play.write_text(textwrap.dedent(f"""\
            - hosts: localhost
              connection: local
              gather_facts: false
              vars:
                lan_coredns_host_zone_route_enabled: {str(enabled).lower()}
                lan_coredns_resolved_dropin_dir: {self.dropins}
                lan_coredns_bind_ip: {BIND}
                lan_coredns_port: {port}
                lan_coredns_zone_name: {ZONE}
                lan_coredns_smoke_test_name: {NAME}
                lan_coredns_smoke_test_expect: {BIND}
              tasks:
                - ansible.builtin.import_tasks: {TASKS}
            """))
        env = dict(os.environ, PATH=self.path, NB_GETENT_BROKEN="1" if broken else "0",
                   ANSIBLE_LOCALHOST_WARNING="false", ANSIBLE_INVENTORY_UNPARSED_WARNING="false",
                   ANSIBLE_LOCAL_TEMP=str(self.d / "tmp"), ANSIBLE_REMOTE_TEMP=str(self.d / "tmp"))
        return subprocess.run(["ansible-playbook", "-i", "localhost,", str(play)],
                              cwd=self.d, env=env, capture_output=True, text=True, timeout=300)

    def restarts(self):
        return self.calls.read_text().splitlines() if self.calls.exists() else []

    def test_on_routes_only_the_zone_and_checks_the_host_resolves(self):
        r = self.run_tasks(True)
        self.assertEqual(r.returncode, 0, r.stdout[-3000:])
        text = (self.dropins / DROPIN).read_text()
        self.assertIn(f"DNS={BIND}\n", text)
        self.assertIn(f"Domains=~{ZONE}\n", text)
        self.assertNotIn("Domains=~.", text)
        self.assertEqual(self.restarts(), ["restart systemd-resolved"])
        again = self.run_tasks(True)
        self.assertEqual(again.returncode, 0, again.stdout[-3000:])
        self.assertEqual(self.restarts(), ["restart systemd-resolved"], "a second run restarted the resolver")

    def test_a_port_other_than_53_is_named(self):
        r = self.run_tasks(True, port=5300, broken=True)
        self.assertIn(f"DNS={BIND}:5300\n", (self.dropins / DROPIN).read_text())
        self.assertNotEqual(r.returncode, 0)

    def test_a_route_the_host_does_not_use_fails_the_run(self):
        r = self.run_tasks(True, broken=True)
        self.assertNotEqual(r.returncode, 0, r.stdout[-3000:])
        self.assertIn(f"routes {ZONE} to", r.stdout)
        self.assertIn("resolvectl status", r.stdout)

    def test_off_removes_the_route(self):
        self.assertEqual(self.run_tasks(True).returncode, 0)
        r = self.run_tasks(False)
        self.assertEqual(r.returncode, 0, r.stdout[-3000:])
        self.assertFalse((self.dropins / DROPIN).exists())
        self.assertEqual(self.restarts(), ["restart systemd-resolved"] * 2)
        r = self.run_tasks(False)
        self.assertEqual(r.returncode, 0, r.stdout[-3000:])
        self.assertEqual(len(self.restarts()), 2, "removing nothing restarted the resolver")


class GoatliteTurnsItOn(unittest.TestCase):

    def test_a_goatlite_renders_the_route_on(self):
        spec = importlib.util.spec_from_file_location("goatlite_render", ROOT / "ops/scripts/goatlite-render.py")
        render = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = render
        spec.loader.exec_module(render)
        d = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, d)
        pub = d / "key.pub"
        pub.write_text("placeholder: load_config only checks that the file exists\n")
        cfg = d / "goatlite.yaml"
        cfg.write_text(yaml.safe_dump({"schema": "goatbuilder/v1", "preset": "lite",
                                       "lite": {"name": "fixture", "host": BIND, "ssh_user": "ubuntu",
                                                "admin": "admin@example.org", "ssh_pubkey": str(pub)}}))
        gv = yaml.safe_load(render.vars_yml(render.facts(render.load_config(cfg, "fixture"))))
        self.assertIs(gv.get("lan_coredns_host_zone_route_enabled"), True)
        self.assertEqual(gv.get("lan_bind_ip"), BIND)


if __name__ == "__main__":
    unittest.main()
