"""coordinated-upgrade.yml migrates each site's database in turn before starting its services.

Spock replicates rows, not schema changes, so a NetBird upgrade that adds a column must run
its migration at every site. The playbook migrates the migrator first; then each other
site, one at a time, places the migration marker (ops/netbird/entrypoint.sh turns
AutoMigrate on when it is present), starts management, waits for it, removes the marker,
and waits until its Spock subscriptions replicate again, before the next site starts.

Two checks:
  * the order of the plays and of each play's tasks, read from the YAML (always runs);
  * ansible-playbook --syntax-check (skipped, and saying so, when ansible-playbook is not
    on PATH).
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

import yaml

HERE = Path(__file__).resolve().parent
ANSIBLE = HERE.parents[1]
PLAYBOOK = Path(os.environ.get("COORDINATED_UPGRADE_PLAYBOOK", ANSIBLE / "coordinated-upgrade.yml"))
MARKER = "/opt/mgmt-host/management-a/marker/migrations.allow"


def plays():
    return yaml.safe_load(PLAYBOOK.read_text())


def kind(task):
    """What a task does, in the terms the order is checked in."""
    f = task.get("ansible.builtin.file") or {}
    cmd = str((task.get("ansible.builtin.command") or {}).get("cmd", ""))
    if f.get("path") == MARKER and f.get("state") == "touch":
        return "marker-on"
    if f.get("path") == MARKER and f.get("state") == "absent":
        return "marker-off"
    if "up -d management-a" in cmd:
        return "start"
    if "ansible.builtin.wait_for" in task:
        return "wait"
    if "spock.sub_show_status()" in cmd and task.get("until"):
        return "replicating"
    return None


def order(play):
    return [k for k in (kind(t) for t in play.get("tasks") or []) if k]


class Order(unittest.TestCase):
    def setUp(self):
        self.plays = plays()
        self.names = [p["name"] for p in self.plays]

    def play(self, hosts):
        found = [p for p in self.plays if p.get("hosts") == hosts and order(p)[:1] in (["marker-on"], ["start"])]
        self.assertEqual(len(found), 1, f"one play that starts management on {hosts}: {self.names}")
        return found[0]

    def test_the_migrator_migrates_first(self):
        mig = self.play("{{ migrator_host }}")
        self.assertEqual(order(mig), ["marker-on", "start", "wait", "marker-off"])

    def test_each_other_site_migrates_its_own_database_before_the_next_starts(self):
        mig = self.play("{{ migrator_host }}")
        rest = self.play("mgmt_hosts:!{{ migrator_host }}")
        self.assertLess(self.plays.index(mig), self.plays.index(rest), "the migrator's play runs first")
        self.assertEqual(rest.get("serial"), 1, "one site at a time")
        self.assertEqual(order(rest), ["marker-on", "start", "wait", "marker-off", "replicating"],
                         "each site: marker, start (AutoMigrate), wait, marker removed, replication again")

    def test_no_site_starts_without_the_marker_in_the_upgrade(self):
        for p in self.plays:
            o = order(p)
            if "start" in o:
                self.assertIn("marker-on", o[:o.index("start")], f"{p['name']}: started without the marker")


class SyntaxCheck(unittest.TestCase):
    def test_syntax_check(self):
        if not shutil.which("ansible-playbook"):
            self.skipTest("ansible-playbook is not on PATH")
        with tempfile.TemporaryDirectory() as t:
            inv = Path(t) / "hosts.ini"
            inv.write_text("[mgmt_hosts]\nsite-a\nsite-b\nsite-c\n")
            r = subprocess.run(["ansible-playbook", "--syntax-check", "-i", str(inv), str(PLAYBOOK),
                                "-e", "migrator_host=site-a", "-e", "target_image=example"],
                               capture_output=True, text=True, cwd=ANSIBLE)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)


if __name__ == "__main__":
    sys.exit(unittest.main(verbosity=2))
