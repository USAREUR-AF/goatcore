#!/usr/bin/env python3
"""mesh-enroll-host.yml mints a node's key with the groups its type declares.

join-node phase 10 passes `mesh_enroll_group_names` for a machine whose type
file declares `mesh_groups:` (break-glass, network-jump). The play reads those
groups by name from the NetBird database on the minting host, refuses before any
key exists unless each is there exactly once, and mints with their ids, never
with the founder peer's groups.

Runs the play's own tasks for that branch, and its own mint argument list,
against localhost with the database answer given as a registered result.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import jinja2
import yaml

ROOT = Path(__file__).resolve().parents[1]
PLAY = ROOT / "ops/ansible/mesh-enroll-host.yml"
BLOCK = "mesh-enroll | the groups the node's type declares, by name"
MINT = "mesh-enroll | mint a single-use setup key (one use, one hour)"


def walk(tasks):
    for t in tasks or []:
        yield t
        for key in ("block", "rescue", "always"):
            yield from walk(t.get(key))


def task(name):
    for play in yaml.safe_load(PLAY.read_text()):
        for t in walk(play.get("tasks")):
            if t.get("name") == name:
                return t
    raise AssertionError(f"no task named {name!r}")


def require_ansible_playbook(case):
    """Skip without ansible-playbook, unless GOAT_TEST_REQUIRE_ANSIBLE=1 (CI sets
    it), where its absence fails the test instead of skipping it quietly."""
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


class Query(unittest.TestCase):
    def test_the_groups_are_read_by_name_in_the_account(self):
        q = next(t for t in task(BLOCK)["block"] if t["name"] == "mesh-enroll | read the declared groups' ids")["ansible.builtin.command"]["argv"][-1]
        sql = jinja2.Environment().from_string(q).render(_me_accounts={"stdout_lines": ["acc_zagros"]}, mesh_enroll_group_names=["ZAGROS-BREAKGLASS", "ZAGROS-NETWORK"])
        self.assertEqual(" ".join(sql.split()), "SELECT name || '|' || id FROM groups WHERE account_id = 'acc_zagros' AND name IN ('ZAGROS-BREAKGLASS', 'ZAGROS-NETWORK') ORDER BY name")


class DeclaredGroups(unittest.TestCase):
    def setUp(self):
        require_ansible_playbook(self)
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def run_branch(self, names, rows, from_peer=False):
        """The play's assertions for the declared groups, with the database's
        answer given, then its mint argument list rendered."""
        block = task(BLOCK)["block"]
        checks = [t for t in block if "ansible.builtin.assert" in t]
        mint = task(MINT)["ansible.builtin.command"]["argv"]
        out = self.dir / "argv.json"
        play = [{
            "hosts": "localhost", "gather_facts": False, "become": False,
            "vars": {"mesh_enroll_group_names": names, "mesh_enroll_groups_from_peer": from_peer,
                     "_me_accounts": {"stdout_lines": ["acc_zagros"]}, "_me_named": {"stdout_lines": rows},
                     "_me_groups": {"stdout_lines": ["from-the-founder-peer"]}},
            "tasks": checks + [{"ansible.builtin.copy": {"dest": str(out), "content": "{{ (" + mint.strip().removeprefix("{{").removesuffix("}}") + ") | to_json }}"}}],
        }]
        pb = self.dir / "play.yml"
        pb.write_text(yaml.safe_dump(play, sort_keys=False))
        r = subprocess.run(["ansible-playbook", "-i", "localhost,", "-c", "local", str(pb), "-e", "ansible_python_interpreter={{ ansible_playbook_python }}",
                            "-e", "inventory_hostname=node-x"],
                           capture_output=True, text=True, env=dict(os.environ, ANSIBLE_NOCOLOR="1"), cwd=self.dir, timeout=120)
        return r, (json.loads(out.read_text()) if r.returncode == 0 else None)

    def test_the_key_carries_exactly_the_declared_groups(self):
        r, argv = self.run_branch(["ZAGROS-BREAKGLASS"], ["ZAGROS-BREAKGLASS|g-bg"])
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(argv[argv.index("--auto-groups") + 1], "g-bg")
        self.assertNotIn("from-the-founder-peer", argv)

    def test_one_of_two_declared_groups_missing_mints_nothing(self):
        r, _ = self.run_branch(["ZAGROS-BREAKGLASS", "ZAGROS-NETWORK"], ["ZAGROS-BREAKGLASS|g-bg"])
        self.assertNotEqual(r.returncode, 0, r.stdout)

    def test_a_breakglass_node_resolves_on_loopback(self):
        up = task("mesh-enroll | netbird up against the declared URL")["ansible.builtin.command"]["argv"]
        self.assertIn("'--dns-resolver-address', '127.0.0.1:53'", up)
        self.assertIn("'BREAKGLASS' in (mesh_enroll_group_names", up)

    def test_a_declared_group_the_account_lacks_mints_nothing(self):
        r, _ = self.run_branch(["ZAGROS-BREAKGLASS"], [])
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("No key was minted", r.stdout)

    def test_names_that_are_not_group_names_mint_nothing(self):
        r, _ = self.run_branch(["ZAGROS-INFRA') OR ('1'='1"], [])
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("not a group name", r.stdout)

    def test_both_sources_of_groups_mint_nothing(self):
        r, _ = self.run_branch(["ZAGROS-BREAKGLASS"], ["ZAGROS-BREAKGLASS|g-bg"], from_peer=True)
        self.assertNotEqual(r.returncode, 0, r.stdout)


if __name__ == "__main__":
    unittest.main()
