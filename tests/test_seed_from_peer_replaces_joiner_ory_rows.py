#!/usr/bin/env python3
"""A joining node's seed replaces the rows its own identity service wrote at
start, so the seed equals the founder's.

join-node phase 13 deploys Ory on a fresh joiner and then seeds it from the
founder, data-only. Hydra writes its own signing keys when it first starts, so
before this the joiner's hydra_jwk held the founder's keys and its own, and the
check that the seed is current refused (GB-367). The seed's first statements,
ops/ansible/templates/seed-from-peer-begin.sql.j2, now empty the identity
databases' seed tables on every seed, and the netbird tables only when forced.

Renders the real template through Ansible for each database the play seeds,
then, where docker is present, runs it against a Postgres whose hydra_jwk holds
a key of the joiner's own and loads the founder's two.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import uuid

import yaml

ROOT = Path(__file__).resolve().parents[1]
BEGIN = ROOT / "ops/ansible/templates/seed-from-peer-begin.sql.j2"
END = ROOT / "ops/ansible/templates/seed-from-peer-end.sql"
PLAY = ROOT / "ops/ansible/site-seed-from-peer.yml"


def seed_databases():
    """The play's own per-database seed sets, its table lists resolved."""
    play = yaml.safe_load(PLAY.read_text())[0]
    dbs = play["vars"]["seed_databases"]
    defaults = yaml.safe_load((ROOT / "ops/ansible/roles/spock-mesh-peering/defaults/main.yml").read_text())
    for d in dbs:
        if isinstance(d["tables"], str):
            d["tables"] = defaults["spock_mesh_netbird_mgmt_tables"]
    return dbs


def require_ansible_playbook(case):
    """Skip without ansible-playbook, unless GOAT_TEST_REQUIRE_ANSIBLE=1 (CI sets
    it), where its absence fails the test instead of skipping it quietly."""
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


def render(tmp, item, force):
    out = Path(tmp) / f"begin-{item['db']}-{force}.sql"
    play = [{"hosts": "localhost", "gather_facts": False, "vars": {"force_reseed": force, "item": item},
             "tasks": [{"ansible.builtin.template": {"src": str(BEGIN), "dest": str(out), "mode": "0600"}}]}]
    pb = Path(tmp) / "render.yml"
    pb.write_text(yaml.safe_dump(play))
    r = subprocess.run(["ansible-playbook", "-i", "localhost,", "-c", "local", str(pb),
                        "-e", "ansible_python_interpreter={{ ansible_playbook_python }}"],
                       capture_output=True, text=True, timeout=120, env=dict(os.environ, ANSIBLE_NOCOLOR="1"))
    assert r.returncode == 0, r.stdout + r.stderr
    return out.read_text()


class Rendered(unittest.TestCase):
    def setUp(self):
        require_ansible_playbook(self)
        self._tmp = tempfile.TemporaryDirectory()

    def tearDown(self):
        self._tmp.cleanup()

    def test_the_identity_databases_are_emptied_on_every_seed(self):
        for item in seed_databases():
            for force in (False, True):
                with self.subTest(db=item["db"], force=force):
                    sql = render(self._tmp.name, item, force)
                    deletes = [t for t in item["tables"] if f'DELETE FROM public."{t}";' in sql]
                    if force or item["db"] != "netbird":
                        self.assertEqual(deletes, item["tables"])
                        self.assertIn("already registered Spock node", sql)
                    else:
                        self.assertEqual(deletes, [])
                        self.assertNotIn("DELETE FROM", sql)

    def test_hydra_and_kratos_are_seeded(self):
        self.assertEqual(sorted(d["db"] for d in seed_databases()), ["hydra", "kratos", "netbird"])


@unittest.skipUnless(shutil.which("docker"), "docker is not installed")
class AgainstPostgres(unittest.TestCase):
    """The joiner's own key is gone and the founder's two are there after the seed."""

    def setUp(self):
        require_ansible_playbook(self)
        self._tmp = tempfile.TemporaryDirectory()
        self.name = f"seed-test-{uuid.uuid4().hex[:8]}"
        r = subprocess.run(["docker", "run", "-d", "--rm", "--name", self.name, "-e", "POSTGRES_PASSWORD=fixture", "postgres:16-alpine"],
                           capture_output=True, text=True, timeout=300)
        if r.returncode != 0:
            self.skipTest(f"no postgres container: {r.stderr.strip()[:200]}")
        for _ in range(60):
            if subprocess.run(["docker", "exec", self.name, "pg_isready", "-U", "postgres"], capture_output=True).returncode == 0:
                break
            subprocess.run(["sleep", "1"])

    def tearDown(self):
        subprocess.run(["docker", "rm", "-f", self.name], capture_output=True)
        self._tmp.cleanup()

    def psql(self, sql):
        return subprocess.run(["docker", "exec", "-i", "-u", "postgres", self.name, "psql", "-X", "-q", "-v", "ON_ERROR_STOP=1", "-At", "-U", "postgres"],
                              input=sql, capture_output=True, text=True, timeout=60)

    def test_a_fresh_joiners_own_signing_key_does_not_survive_the_seed(self):
        item = next(d for d in seed_databases() if d["db"] == "hydra")
        schema = "CREATE TABLE hydra_client (id text PRIMARY KEY);\nCREATE TABLE hydra_jwk (pk serial PRIMARY KEY, sid text, kid text);\n"
        self.assertEqual(self.psql(schema + "INSERT INTO hydra_jwk (sid, kid) VALUES ('hydra.openid.id-token', 'joiner-own');").returncode, 0)
        dump = "INSERT INTO public.hydra_jwk (pk, sid, kid) VALUES (101, 'hydra.openid.id-token', 'founder-1'), (102, 'hydra.jwt.access-token', 'founder-2');\n"
        r = self.psql(render(self._tmp.name, item, False) + dump + END.read_text())
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        rows = self.psql("SELECT kid FROM hydra_jwk ORDER BY kid;").stdout.split()
        self.assertEqual(rows, ["founder-1", "founder-2"])


if __name__ == "__main__":
    unittest.main()
