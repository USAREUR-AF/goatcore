#!/usr/bin/env python3
"""A joining node's seed replaces the rows its own identity service wrote at
start, so the seed equals the founder's.

join-node phase 13 deploys Ory on a fresh joiner and then seeds it from the
founder, data-only. Hydra writes its own signing keys when it first starts, so
before this the joiner's hydra_jwk held the founder's keys and its own, and the
check that the seed is current refused (GB-367). The seed's first statements,
ops/ansible/templates/seed-from-peer-begin.sql.j2, now empty hydra_jwk on every
seed, and every seed table only when forced.

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


def wait_for_postgres(name, limit=90, run=subprocess.run, clock=None, sleep=None):
    """Wait until the container's Postgres answers and keeps answering.

    The postgres image runs a temporary server while it initialises, and
    pg_isready answers it; that server then stops and the real one starts, so a
    query in between finds no server (psql exits 2). So pg_isready first, then
    `psql -c 'SELECT 1'` until it succeeds twice in a row; a failure between
    them starts the count again. Gives up after `limit` seconds."""
    import time
    clock = clock or time.monotonic
    sleep = sleep or time.sleep
    end = clock() + limit

    def ok(argv):
        return run(["docker", "exec", name] + argv, capture_output=True).returncode == 0

    while not ok(["pg_isready", "-U", "postgres"]):
        if clock() >= end:
            return False
        sleep(1)
    in_a_row = 0
    while in_a_row < 2:
        in_a_row = in_a_row + 1 if ok(["psql", "-X", "-U", "postgres", "-c", "SELECT 1"]) else 0
        if in_a_row < 2:
            if clock() >= end:
                return False
            sleep(1)
    return True


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

    def test_hydra_jwk_is_emptied_on_every_seed_and_the_rest_only_when_forced(self):
        for item in seed_databases():
            for force in (False, True):
                with self.subTest(db=item["db"], force=force):
                    sql = render(self._tmp.name, item, force)
                    deletes = [t for t in item["tables"] if f'DELETE FROM public."{t}";' in sql]
                    want = item["tables"] if force else (["hydra_jwk"] if item["db"] == "hydra" else [])
                    self.assertEqual(deletes, want)
                    if want:
                        self.assertIn("already registered Spock node", sql)
                    else:
                        self.assertNotIn("DELETE FROM", sql)

    def test_every_seed_refuses_a_spock_node_before_it_changes_anything(self):
        play = yaml.safe_load(PLAY.read_text())[0]
        names = [t.get("name", "") for t in play["tasks"]]
        probe = next(t for t in play["tasks"] if t.get("name") == "seed | probe whether this node is already a Spock node")
        refuse = next(t for t in play["tasks"] if t.get("name") == "seed | refuse to seed a node that is already a Spock node")
        self.assertNotIn("when", probe)
        self.assertNotIn("when", refuse)
        self.assertLess(names.index(refuse["name"]), next(i for i, t in enumerate(play["tasks"]) if "block" in t))

    def test_hydra_and_kratos_are_seeded(self):
        self.assertEqual(sorted(d["db"] for d in seed_databases()), ["hydra", "kratos", "netbird"])


class WaitForPostgres(unittest.TestCase):
    """The wait holds for a server that answers pg_isready and then restarts.

    A stand-in for `docker exec`: pg_isready answers at once, as it does for the
    image's temporary server; psql then finds no server for a while, answers
    once, drops again, and then stays up."""

    def stand_in(self, psql):
        calls = []
        t = [0.0]

        def run(argv, capture_output=True):
            calls.append(argv)
            rc = 0 if "pg_isready" in argv else psql.pop(0) if psql else 0
            return subprocess.CompletedProcess(argv, rc)

        def sleep(s):
            t[0] += s
        return run, (lambda: t[0]), sleep, calls

    def test_a_restart_after_pg_isready_is_waited_out(self):
        run, clock, sleep, calls = self.stand_in([2, 2, 0, 2, 0, 0])
        self.assertTrue(wait_for_postgres("pg", run=run, clock=clock, sleep=sleep))
        queries = [c for c in calls if "psql" in c]
        # Not before two answers in a row: the lone answer between the drops does not count.
        self.assertEqual(len(queries), 6, calls)
        self.assertEqual(queries[-1], ["docker", "exec", "pg", "psql", "-X", "-U", "postgres", "-c", "SELECT 1"])

    def test_a_server_that_never_holds_gives_up_at_the_limit(self):
        run, clock, sleep, calls = self.stand_in([0, 2] * 100)
        self.assertFalse(wait_for_postgres("pg", limit=10, run=run, clock=clock, sleep=sleep))
        self.assertLessEqual(clock(), 11)


@unittest.skipUnless(shutil.which("docker"), "docker is not installed")
class AgainstPostgres(unittest.TestCase):
    """The joiner's own key is gone and the founder's two are there after the seed."""

    def setUp(self):
        require_ansible_playbook(self)
        self._tmp = tempfile.TemporaryDirectory()
        self.name = f"seed-test-{uuid.uuid4().hex[:8]}"
        # The image the seed-retry regression pins, with no network.
        image = "postgres:16@sha256:ca0bd484cb98bf4b24eb1010e73fb3fcbd6714d240fbc1a10eea5b7dbecb641d"
        r = subprocess.run(["docker", "run", "-d", "--rm", "--network", "none", "--name", self.name, "-e", "POSTGRES_PASSWORD=fixture", image],
                           capture_output=True, text=True, timeout=600)
        self.assertEqual(r.returncode, 0, r.stderr)
        if not wait_for_postgres(self.name):
            self.fail("postgres did not become ready")

    def tearDown(self):
        subprocess.run(["docker", "rm", "-f", self.name], capture_output=True)
        self._tmp.cleanup()

    def psql(self, sql):
        return subprocess.run(["docker", "exec", "-i", "-u", "postgres", self.name, "psql", "-X", "-q", "-v", "ON_ERROR_STOP=1", "-At", "-U", "postgres"],
                              input=sql, capture_output=True, text=True, timeout=60)

    def test_an_unforced_kratos_seed_deletes_nothing(self):
        item = next(d for d in seed_databases() if d["db"] == "kratos")
        tables = "".join(f'CREATE TABLE "{t}" (id text PRIMARY KEY);\n' for t in item["tables"])
        self.assertEqual(self.psql(tables + "INSERT INTO identities (id) VALUES ('local');").returncode, 0)
        r = self.psql(render(self._tmp.name, item, False) + "INSERT INTO public.identities (id) VALUES ('founder');\n" + END.read_text())
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.psql("SELECT id FROM identities ORDER BY id;").stdout.split(), ["founder", "local"])

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
