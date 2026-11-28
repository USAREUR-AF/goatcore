#!/usr/bin/env python3
"""An empty slot-fragment file is refused instead of rendering every slot
unassigned.

Runs the goat-zenoh role's own slot-fragment tasks, taken unchanged from
ops/ansible/roles/goat-zenoh/tasks/main.yml (read the fragment, the refusals,
carry it, its digest check), against localhost with the fragment in a temporary
directory. The role's defaults are loaded under the test's own values, as the
role loads them, so a task that reads a role default runs as it does in the role.
"""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
ROLE_TASKS = ROOT / "ops/ansible/roles/goat-zenoh/tasks/main.yml"
ROLE_DEFAULTS = ROOT / "ops/ansible/roles/goat-zenoh/defaults/main.yml"
SALT = "zagros-fixture-salt"
POOL = 3


def anchor(k):
    return hashlib.sha256(f"{SALT}:{k}".encode()).hexdigest()[:32]


def fragment():
    """A fragment as the portal writes it: a digest line, then every pool slot."""
    body = "".join(f'      {{\n        id: "slot-{anchor(k)}",\n        cert_common_names: ["{anchor(k)}.unassigned"],\n      }},\n' for k in range(POOL))
    return f"      // slot fragment sha256:{hashlib.sha256(body.encode()).hexdigest()}\n" + body


def fragment_tasks():
    """The role's slot-fragment tasks, in order and unchanged."""
    tasks, out = yaml.safe_load(ROLE_TASKS.read_text()), []
    start = next(i for i, t in enumerate(tasks) if t.get("name") == "GHSA-rr3p | read the portal's slot fragment")
    for t in tasks[start:]:
        out.append(t)
        if "digest matches its content" in str(t.get("name", "")):
            return out
    raise AssertionError("the role's digest check follows its read of the fragment")


def require_ansible_playbook(case):
    """Skip without ansible-playbook, unless GOAT_TEST_REQUIRE_ANSIBLE=1 (CI sets
    it), where its absence fails the test instead of skipping it quietly."""
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


class EmptyFragment(unittest.TestCase):
    def setUp(self):
        require_ansible_playbook(self)
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)
        self.fragment = self.dir / "slot-fragment.json5"

    def tearDown(self):
        self._tmp.cleanup()

    def run_role(self, extra=()):
        play = [{
            "hosts": "router-a", "gather_facts": False, "become": False,
            "vars": {
                **yaml.safe_load(ROLE_DEFAULTS.read_text()),
                "goat_zenoh_slot_fragment_path": str(self.fragment),
                "zenoh_slot_fragment_bootstrap": False,
                "zenoh_slot_pool_salt": SALT,
                "zenoh_slot_pool_size": POOL,
                "jz_per_vendor": False,
            },
            "tasks": fragment_tasks(),
        }]
        pb = self.dir / "play.yml"
        pb.write_text(yaml.safe_dump(play, sort_keys=False))
        inv = self.dir / "inv.ini"
        inv.write_text("router-a ansible_connection=local ansible_python_interpreter='{{ ansible_playbook_python }}'\n")
        env = dict(os.environ, ANSIBLE_NOCOLOR="1", ANSIBLE_RETRY_FILES_ENABLED="0")
        return subprocess.run(["ansible-playbook", "-i", str(inv), str(pb), *extra], capture_output=True, text=True, env=env, cwd=self.dir, timeout=180)

    def test_a_portal_written_fragment_passes(self):
        self.fragment.write_text(fragment())
        r = self.run_role()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_an_empty_fragment_is_refused(self):
        for content in ("", "\n", "  \n\t\n"):
            with self.subTest(repr(content)):
                self.fragment.write_text(content)
                r = self.run_role()
                self.assertNotEqual(r.returncode, 0, r.stdout)
                self.assertIn("is empty", r.stdout)

    def test_an_empty_fragment_is_refused_even_with_the_bootstrap_switch(self):
        self.fragment.write_text("")
        r = self.run_role(["-e", "zenoh_slot_fragment_bootstrap=true"])
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("is empty", r.stdout)

    def test_no_fragment_with_the_bootstrap_switch_still_renders(self):
        r = self.run_role(["-e", "zenoh_slot_fragment_bootstrap=true"])
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_a_fragment_whose_digest_does_not_match_is_refused(self):
        self.fragment.write_text(fragment().replace(".unassigned", ".g2", 1))
        r = self.run_role()
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("is not usable", r.stdout)


if __name__ == "__main__":
    unittest.main()
