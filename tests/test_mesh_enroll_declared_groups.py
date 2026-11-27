#!/usr/bin/env python3
"""mesh-enroll-host.yml mints a node's key with the groups its type declares.

join-node phase 10 passes `mesh_enroll_group_names` for a machine whose type
file declares `mesh_groups:` (break-glass). The play reads those groups by name
from the NetBird database on the minting host, refuses before any key exists
unless each is there exactly once, and mints with their ids, never with the
founder peer's groups. A node already enrolled is refused unless its peer holds
exactly those groups.

Runs the play's own tasks for that branch, and its own mint argument list,
against localhost with the database answer given as a registered result.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

import jinja2
import yaml

ROOT = Path(__file__).resolve().parents[1]
PLAY = ROOT / "ops/ansible/mesh-enroll-host.yml"
BLOCK = "mesh-enroll | the groups the node's type declares, by name"
MINT = "mesh-enroll | mint a single-use setup key (one use, one hour)"
ENROLLED = "mesh-enroll | an enrolled node holds exactly the groups its type declares"
KEY = "fixture-wireguard-private-key-not-a-secret"
READ = "mesh-enroll | the enrolled node's active profile and resolver address"


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
        sql = jinja2.Environment(autoescape=jinja2.select_autoescape(default_for_string=False)).from_string(q).render(_me_accounts={"stdout_lines": ["acc_zagros"]}, mesh_enroll_group_names=["ZAGROS-BREAKGLASS", "ZAGROS-NETWORK"])
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

    def read_resolver(self, cfg_new=None, cfg_old=None, active=None):
        """The play's own on-node read, run here against files laid out as
        given (None: absent; a str: the file's raw text)."""
        argv = next(t for t in task(ENROLLED)["block"] if t["name"] == READ)["ansible.builtin.command"]["argv"]
        self.assertEqual(argv[3:], ["/var/lib/netbird", "/var/lib/netbird/default.json", "/etc/netbird/config.json"])
        state, old = self.dir / "state", self.dir / "etc-config.json"
        state.mkdir(exist_ok=True)
        for path, content in ((state / "default.json", cfg_new), (old, cfg_old), (state / "active_profile.json", active)):
            if path.exists():
                path.unlink()
            if content is not None:
                path.write_text(content if isinstance(content, str) else json.dumps(content))
        r = subprocess.run([sys.executable, "-I", "-c", argv[2], str(state), str(state / "default.json"), str(old)],
                           capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertNotIn(KEY, r.stdout + r.stderr)
        return r.stdout

    def run_enrolled(self, names, held, ip="100.127.0.30/16", cfg_new=None, cfg_old=None, active=None):
        """The play's checks for a node that is already enrolled, with its own
        status, its client configuration files and the database's answers."""
        b = task(ENROLLED)
        checks = [t for t in b["block"] if "ansible.builtin.command" not in t]
        self.assertEqual(sum("ansible.builtin.assert" in t for t in checks), 4)
        play = [{
            "hosts": "localhost", "gather_facts": False, "become": False,
            "vars": dict(b["vars"], **{"mesh_enroll_group_names": names, "_me_first_host": "mgmt-zagros-01",
                     "_me_status": {"stdout": json.dumps({"netbirdIp": ip, "management": {"connected": True}})},
                     "_me_held_accounts": {"stdout_lines": ["acc_zagros"]}, "_me_held": {"stdout_lines": held},
                     "_me_resolver_read": {"stdout": self.read_resolver(cfg_new, cfg_old, active)}}),
            "tasks": checks,
        }]
        pb = self.dir / "enrolled.yml"
        pb.write_text(yaml.safe_dump(play, sort_keys=False))
        return subprocess.run(["ansible-playbook", "-i", "localhost,", "-c", "local", str(pb), "-e", "ansible_python_interpreter={{ ansible_playbook_python }}",
                               "-e", "inventory_hostname=node-x"],
                              capture_output=True, text=True, env=dict(os.environ, ANSIBLE_NOCOLOR="1"), cwd=self.dir, timeout=120)

    def test_an_enrolled_node_in_exactly_its_declared_groups_passes(self):
        loop = {"PrivateKey": KEY, "CustomDNSAddress": "127.0.0.1:53"}
        r = self.run_enrolled(["ZAGROS-BREAKGLASS"], ["ZAGROS-BREAKGLASS"], cfg_new=loop)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertNotIn(KEY, r.stdout + r.stderr)
        # The older single file is read when the default profile is absent,
        # and an active profile named default is the default profile.
        r = self.run_enrolled(["ZAGROS-BREAKGLASS"], ["ZAGROS-BREAKGLASS"], cfg_old=loop, active={"name": "default", "username": ""})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_the_resolver_is_read_from_the_file_the_client_uses(self):
        # The default profile wins over the older file; the older file is
        # read only when the default profile is absent.
        out = json.loads(self.read_resolver({"CustomDNSAddress": ""}, {"CustomDNSAddress": "127.0.0.1:53"}))
        self.assertEqual((out["resolver"], Path(out["file"]).name), ("", "default.json"))
        out = json.loads(self.read_resolver(None, {"CustomDNSAddress": "127.0.0.1:53"}))
        self.assertEqual((out["resolver"], Path(out["file"]).name), ("127.0.0.1:53", "etc-config.json"))
        self.assertEqual(json.loads(self.read_resolver(None, None)), {"profile": "default", "file": "", "resolver": ""})
        self.assertEqual(json.loads(self.read_resolver("{not json " + KEY, None))["resolver"], "(unreadable)")
        self.assertEqual(json.loads(self.read_resolver(None, None, active={"name": "bg", "username": "root"}))["profile"], "bg")

    def test_an_enrolled_breakglass_node_off_loopback_is_refused(self):
        # ADR 1114 §2: enrolled by hand with the right group but without
        # --dns-resolver-address, or with another address, or with no client
        # configuration, or an unreadable one, or another active profile (whose
        # file this check does not read): each is refused, the key never printed.
        loop = {"PrivateKey": KEY, "CustomDNSAddress": "127.0.0.1:53"}
        for new, old, active in (({"PrivateKey": KEY}, None, None), ({"PrivateKey": KEY, "CustomDNSAddress": "100.127.0.30:53"}, None, None),
                                 (None, None, None), ({"PrivateKey": KEY, "CustomDNSAddress": ""}, loop, None),
                                 ("{truncated " + KEY, None, None), (loop, None, {"name": "bg", "username": "root"})):
            r = self.run_enrolled(["ZAGROS-BREAKGLASS"], ["ZAGROS-BREAKGLASS"], cfg_new=new, cfg_old=old, active=active)
            self.assertNotEqual(r.returncode, 0, (new, old, active))
            self.assertIn("resolves on loopback", r.stdout, (new, old, active))
            self.assertNotIn(KEY, r.stdout + r.stderr)

    def test_an_enrolled_node_of_another_group_needs_no_loopback_resolver(self):
        r = self.run_enrolled(["ZAGROS-NETWORK"], ["ZAGROS-NETWORK"])
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_an_enrolled_node_in_other_groups_is_refused(self):
        # Enrolled by hand with the founder peer's groups, or with no group but
        # All, or with the declared group and INFRA too: each is refused.
        for held in (["ZAGROS-INFRA"], [], ["ZAGROS-BREAKGLASS", "ZAGROS-INFRA"]):
            r = self.run_enrolled(["ZAGROS-BREAKGLASS"], held)
            self.assertNotEqual(r.returncode, 0, held)
            self.assertIn("is already enrolled", r.stdout, held)

    def test_the_enrolled_check_runs_only_for_an_enrolled_node_with_declared_groups(self):
        when = task(ENROLLED)["when"]
        self.assertEqual(when, ["_me_enrolled | bool", "(mesh_enroll_group_names | default([]) | length) > 0"])
        self.assertIn("p.ip = '\"{{ _me_own_ip }}\"'", next(t for t in task(ENROLLED)["block"] if t["name"] == "mesh-enroll | read the groups the enrolled node's peer holds")["ansible.builtin.command"]["argv"][-1])

    def render_up(self, extra):
        """The play's own `netbird up` argument list, rendered by ansible with
        the given variables."""
        up = task("mesh-enroll | netbird up against the declared URL")["ansible.builtin.command"]["argv"]
        out = self.dir / "up.json"
        play = [{
            "hosts": "localhost", "gather_facts": False, "become": False,
            "vars": dict({"mesh_enrollment_url": "https://mesh.zagros.example.org"}, **extra),
            "tasks": [{"ansible.builtin.copy": {"dest": str(out), "content": "{{ (" + up.strip().removeprefix("{{").removesuffix("}}") + ") | to_json }}"}}],
        }]
        pb = self.dir / "up.yml"
        pb.write_text(yaml.safe_dump(play, sort_keys=False))
        r = subprocess.run(["ansible-playbook", "-i", "localhost,", "-c", "local", str(pb), "-e", "ansible_python_interpreter={{ ansible_playbook_python }}"],
                           capture_output=True, text=True, env=dict(os.environ, ANSIBLE_NOCOLOR="1"), cwd=self.dir, timeout=120)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        return json.loads(out.read_text())

    def test_a_breakglass_node_resolves_on_loopback(self):
        argv = self.render_up({"mesh_enroll_group_names": ["ZAGROS-BREAKGLASS"]})
        self.assertEqual(argv[argv.index("--dns-resolver-address") + 1], "127.0.0.1:53")

    def test_only_a_breakglass_node_resolves_on_loopback(self):
        # A core node (the founder peer's groups, no declared names) and a node
        # in another declared group keep NetBird's own resolver address.
        for extra in ({"mesh_enroll_groups_from_peer": True}, {"mesh_enroll_group_names": ["ZAGROS-NETWORK"]},
                      {"mesh_enroll_group_names": ["ZAGROS-BREAKGLASS-LAB"]}):
            argv = self.render_up(extra)
            self.assertNotIn("--dns-resolver-address", argv, extra)
            self.assertEqual(argv[:2], ["netbird", "up"])

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
