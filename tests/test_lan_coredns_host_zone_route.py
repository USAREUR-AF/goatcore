"""A host that serves its site's LAN zone resolves that zone itself, and nothing else
about its name resolution changes.

The lan-coredns role answers the zone on the host's LAN address, but nothing gave
the host itself the zone's names: on a goatlite's box `goat status` failed its
portal leg (acceptance row goat-status-chain). A first fix routed the zone to the
zone's server with a systemd-resolved drop-in. resolved routes by scope, not by
server, so that put the server in the global scope beside the uplink's; the zone
server answers NXDOMAIN outside the zone, became the current server, and the box
stopped resolving outside names (apt failed in standup phase 10 on a VM founder).
The role now writes the zone file's lines into a marked block of the hosts file,
and removes the earlier drop-in wherever it is.

Each case runs the role's tasks/host-zone-route.yml on localhost, with the hosts
file and the drop-in directory in the test's directory and stand-ins first on
PATH for systemctl (records each call) and getent (reads the test's hosts file,
as the host's resolver would). Become is off; nothing outside the test's
directory is written.

Checked:
  - on: the zone's names (no comment lines) are in a marked block, the rest of
    the hosts file is kept, the host check passes, and no resolver file is written;
  - on, a second run: nothing changes and nothing restarts;
  - on, where the earlier drop-in is present: it is removed, with one restart;
  - on, the host still does not resolve the name: the run fails and says why;
  - off: the block is removed and the rest of the hosts file kept;
  - a goatlite's render turns it on.

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
HOSTS = "127.0.0.1 localhost\n127.0.1.1 mgmt-zagros-01\n"
ZONE_FILE = f"# generated\n{BIND} {NAME} netbird.{ZONE}\n\n{BIND} portal.zagros.{ZONE}\n"


def require_ansible_playbook(case):
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


class HostZoneNames(unittest.TestCase):

    def setUp(self):
        require_ansible_playbook(self)
        self.d = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.d)
        self.hosts = self.d / "hosts"
        self.hosts.write_text(HOSTS)
        self.dropins = self.d / "resolved.conf.d"
        self.dropins.mkdir()
        self.zones = self.d / "dns"
        self.zones.mkdir()
        (self.zones / f"{ZONE}.hosts").write_text(ZONE_FILE)
        self.calls = self.d / "systemctl.calls"
        stubs = self.d / "bin"
        stubs.mkdir()
        (stubs / "systemctl").write_text(f'#!/bin/sh\necho "$*" >> "{self.calls}"\n')
        (stubs / "getent").write_text(textwrap.dedent(f"""\
            #!/bin/sh
            [ "$NB_GETENT_BROKEN" = 1 ] && exit 2
            grep -v '^#' "{self.hosts}" | grep -w -- "$2" | head -1 | grep . || exit 2
            """))
        for p in stubs.iterdir():
            p.chmod(0o755)
        self.path = f"{stubs}:{os.environ.get('PATH', '')}"

    def run_tasks(self, enabled, broken=False):
        play = self.d / "play.yml"
        play.write_text(textwrap.dedent(f"""\
            - hosts: localhost
              connection: local
              gather_facts: false
              vars:
                lan_coredns_host_zone_route_enabled: {str(enabled).lower()}
                lan_coredns_hosts_file: {self.hosts}
                lan_coredns_resolved_dropin_dir: {self.dropins}
                lan_coredns_zone_dir: {self.zones}
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

    def test_on_writes_the_zone_names_and_no_resolver_route(self):
        r = self.run_tasks(True)
        self.assertEqual(r.returncode, 0, r.stdout[-3000:])
        text = self.hosts.read_text()
        self.assertTrue(text.startswith(HOSTS), text)
        self.assertIn(f"{BIND} {NAME} netbird.{ZONE}\n", text)
        self.assertIn(f"{BIND} portal.zagros.{ZONE}\n", text)
        self.assertNotIn("# generated", text)
        self.assertEqual(list(self.dropins.iterdir()), [], "a resolver file was written")
        self.assertEqual(self.restarts(), [])
        again = self.run_tasks(True)
        self.assertEqual(again.returncode, 0, again.stdout[-3000:])
        self.assertEqual(self.hosts.read_text(), text)
        self.assertEqual(self.restarts(), [])

    def test_the_earlier_resolver_route_is_removed(self):
        (self.dropins / DROPIN).write_text(f"[Resolve]\nDNS={BIND}\nDomains=~{ZONE}\n")
        r = self.run_tasks(True)
        self.assertEqual(r.returncode, 0, r.stdout[-3000:])
        self.assertFalse((self.dropins / DROPIN).exists())
        self.assertEqual(self.restarts(), ["restart systemd-resolved"])

    def test_a_block_the_host_does_not_use_fails_the_run(self):
        r = self.run_tasks(True, broken=True)
        self.assertNotEqual(r.returncode, 0, r.stdout[-3000:])
        self.assertIn("nsswitch.conf", r.stdout)

    def test_off_removes_the_block_and_keeps_the_rest(self):
        self.assertEqual(self.run_tasks(True).returncode, 0)
        r = self.run_tasks(False)
        self.assertEqual(r.returncode, 0, r.stdout[-3000:])
        self.assertEqual(self.hosts.read_text(), HOSTS)

    def test_no_task_writes_a_resolver_route(self):
        text = TASKS.read_text()
        self.assertNotIn("[Resolve]", text)
        self.assertNotIn("DNS={{", text)


class GoatliteTurnsItOn(unittest.TestCase):

    def test_a_goatlite_renders_it_on(self):
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


if __name__ == "__main__":
    unittest.main()
