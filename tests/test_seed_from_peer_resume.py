#!/usr/bin/env python3
"""A resumed join-node runs the seed again on a node it already seeded: when
every seed table still equals the founder's, the seed loads nothing and the run
goes on; when one differs, it is refused as before.

Runs site-seed-from-peer.yml's own tasks, from its start through its refusal of
a populated account, through a real ansible-playbook with the founder and the
joiner on this machine, against the stand-in `docker` of
tests/test_spock_seed_current_check.py, which answers from files.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests"))
from test_spock_seed_current_check import STAND_IN  # noqa: E402

SEED_PLAY = ROOT / "ops/ansible/site-seed-from-peer.yml"
REFUSE = "seed | refuse to overwrite a populated account unless -e force_reseed=true"

# The seed play's own first query: the joiner's account count.
ACCOUNTS = r'''
if "count(*) FROM accounts" in sql:
    print(len(rows("accounts")))
    sys.exit(0)
'''
STAND = STAND_IN.replace('if "seed-check-sums" in sql:', ACCOUNTS.strip("\n") + '\nif "seed-check-sums" in sql:', 1)


def require_ansible_playbook(case):
    """Skip without ansible-playbook, unless GOAT_TEST_REQUIRE_ANSIBLE=1 (CI sets
    it), where its absence fails the test instead of skipping it quietly."""
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


TABLES = {"netbird": ["accounts", "users"], "kratos": ["identities"], "hydra": ["hydra_jwk"]}
BASE = {"accounts": ["acc_1|zagros"], "users": ["u1|admin"], "identities": ["id1|me@example.org"], "hydra_jwk": ["k1|id-token"]}


class Resume(unittest.TestCase):
    def setUp(self):
        require_ansible_playbook(self)

    def run_seed(self, founder, joiner, force=False):
        with tempfile.TemporaryDirectory() as t:
            t = Path(t)
            for side, data in (("founder", founder), ("joiner", joiner)):
                (t / side).mkdir()
                for db, tabs in TABLES.items():
                    for tab in tabs:
                        (t / side / f"{db}.{tab}").write_text("".join(r + "\n" for r in data.get(tab, [])))
            (t / "state.json").write_text("{}")
            (t / "bin").mkdir()
            (t / "bin/docker").write_text(STAND)
            (t / "bin/docker").chmod(0o755)
            (t / "hosts.ini").write_text("[gamma_hosts]\njoiner ansible_connection=local\n[zagros_hosts]\nfounder ansible_connection=local\n")
            gv = t / "group_vars/gamma_hosts"
            gv.mkdir(parents=True)
            (gv / "vars.yml").write_text("join_founder_site: zagros\n")
            doc = yaml.safe_load(SEED_PLAY.read_text())
            play = doc[0]
            play["become"] = False
            play.pop("vars_files", None)
            play["vars"]["seed_databases"] = [{"db": db, "tables": tabs} for db, tabs in TABLES.items()]
            play["vars"]["postgres_container"] = "pg-joiner"
            tasks = play["tasks"]
            end = next(i for i, x in enumerate(tasks) if x.get("name") == REFUSE)
            play["tasks"] = tasks[: end + 1] + [{"name": "seed | REACHED THE LOAD", "ansible.builtin.debug": {"msg": "would load"}}]
            for task in play["tasks"]:
                if "ansible.builtin.include_tasks" in task:
                    task["ansible.builtin.include_tasks"] = str(ROOT / "ops/ansible" / task["ansible.builtin.include_tasks"])
                    # The stand-in tells the two sides apart by container name.
                    task.setdefault("vars", {})["peer_postgres_container"] = "pg-founder"
            (t / "play.yml").write_text(yaml.safe_dump(doc))
            env = {k: v for k, v in os.environ.items() if not k.startswith("ANSIBLE_")}
            env.update({"PATH": f"{t / 'bin'}:{env['PATH']}", "SEED_STANDIN": str(t), "ANSIBLE_BECOME": "False",
                        "ANSIBLE_CONFIG": str(t / "none.cfg"), "ANSIBLE_LOCAL_TEMP": str(t / "tmp"),
                        "ANSIBLE_HOME": str(t / "home"), "ANSIBLE_NOCOLOR": "1"})
            r = subprocess.run(["ansible-playbook", "-i", str(t / "hosts.ini"), str(t / "play.yml"),
                                "-e", "site_id=gamma", "-e", f"force_reseed={str(force).lower()}"],
                               cwd=t, env=env, capture_output=True, text=True, timeout=300)
            return r

    def test_a_node_seeded_and_still_current_loads_nothing_and_goes_on(self):
        r = self.run_seed(BASE, BASE)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("is already seeded", r.stdout)
        self.assertNotIn("REACHED THE LOAD", r.stdout)

    def test_a_seeded_node_that_differs_is_refused(self):
        founder = dict(BASE, identities=BASE["identities"] + ["id2|new@example.org"])
        r = self.run_seed(founder, BASE)
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("Refusing to re-seed", r.stdout)
        self.assertIn("kratos.identities", r.stdout)
        self.assertNotIn("new@example.org", r.stdout + r.stderr)

    def test_a_fresh_node_goes_to_the_load_without_comparing(self):
        r = self.run_seed(BASE, {})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("REACHED THE LOAD", r.stdout)
        self.assertNotIn("seed-check 1", r.stdout)

    def test_force_reseed_goes_to_the_load(self):
        r = self.run_seed(BASE, BASE, force=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("REACHED THE LOAD", r.stdout)


if __name__ == "__main__":
    unittest.main()
