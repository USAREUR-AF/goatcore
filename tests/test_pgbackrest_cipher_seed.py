#!/usr/bin/env python3
"""A host's pgBackRest repo1 passphrase survives a rename and is unchanged
for a host that predates the recorded seed.

Runs the real task file (ops/ansible/tasks/pgbackrest-cipher-seed.yml) and the
real derivation from site-bootstrap.yml's paired-stanza play against
localhost, with the seed file in a temporary directory.
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
TASKS = ROOT / "ops/ansible/tasks/pgbackrest-cipher-seed.yml"
PASS = "fixture-cipher-pass"


def derivation():
    """The paired-stanza play's pgbackrest_repo1_cipher_pass expression."""
    plays = yaml.safe_load((ROOT / "ops/ansible/site-bootstrap.yml").read_text())
    play = next(p for p in plays if "pgBackRest paired-stanza" in p.get("name", ""))
    include = [t for t in play["tasks"] if "pgbackrest-cipher-seed.yml" in str(t.get("ansible.builtin.include_tasks", ""))]
    assert include, "the play reads the recorded seed before the role runs"
    assert play["tasks"].index(include[0]) < next(i for i, t in enumerate(play["tasks"]) if "import_role" in str(t) or "ansible.builtin.import_role" in t)
    return play["vars"]["pgbackrest_repo1_cipher_pass"]


def expected(seed):
    return hashlib.sha256(f"{PASS}-repo1-{seed}".encode()).hexdigest()[:32]


@unittest.skipUnless(shutil.which("ansible-playbook"), "ansible-playbook not installed")
class CipherSeed(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)
        self.seed = self.dir / "etc-pgbackrest" / "cipher-seed"

    def tearDown(self):
        self._tmp.cleanup()

    def run_as(self, hostname):
        out = self.dir / "cipher.out"
        play = [{
            "hosts": hostname, "gather_facts": False, "become": False,
            "vars": {
                "pgbackrest_cipher_pass": PASS,
                "pgbackrest_cipher_seed_path": str(self.seed),
                "pgbackrest_cipher_seed_owner": os.getuid(),
                "pgbackrest_cipher_seed_group": os.getgid(),
                "pgbackrest_repo1_cipher_pass": derivation(),
            },
            "tasks": [
                {"ansible.builtin.include_tasks": str(TASKS)},
                {"ansible.builtin.copy": {"dest": str(out), "content": "{{ pgbackrest_repo1_cipher_pass }}"}},
            ],
        }]
        pb = self.dir / "play.yml"
        pb.write_text(yaml.safe_dump(play))
        inv = self.dir / "inv.ini"
        inv.write_text(f"{hostname} ansible_connection=local ansible_python_interpreter='{{{{ ansible_playbook_python }}}}'\n")
        env = dict(os.environ, ANSIBLE_NOCOLOR="1", ANSIBLE_RETRY_FILES_ENABLED="0")
        r = subprocess.run(["ansible-playbook", "-i", str(inv), str(pb)], capture_output=True, text=True, env=env, cwd=self.dir, timeout=180)
        return r, (out.read_text() if r.returncode == 0 else None)

    def test_a_host_without_a_record_keeps_todays_value_and_records_it(self):
        r, cipher = self.run_as("node-old")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(cipher, expected("node-old"))
        self.assertEqual(self.seed.read_text().strip(), "node-old")
        self.assertEqual(self.seed.stat().st_mode & 0o777, 0o600)

    def test_a_renamed_host_keeps_the_passphrase_its_repo_was_made_with(self):
        self.run_as("node-old")
        r, cipher = self.run_as("node-new")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(cipher, expected("node-old"))
        self.assertEqual(self.seed.read_text().strip(), "node-old", "the record is never rewritten")

    def test_an_empty_record_is_refused(self):
        self.seed.parent.mkdir(parents=True)
        self.seed.write_text("\n")
        r, _ = self.run_as("node-new")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("is empty", r.stdout)


if __name__ == "__main__":
    unittest.main()
