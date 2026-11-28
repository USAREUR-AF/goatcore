#!/usr/bin/env python3
"""spock-seed-current-check.yml: a joiner's seed tables still equal its founder's.

join-node phase 13 seeds a joining site from its founder (site-seed-from-peer.yml,
data only), then wires Spock with synchronize_data false. A founder write after
the seed's snapshot and before the subscription exists is never sent. The play
refuses (the code below) when any seed table differs between the founder and
the joiner:

  - before Spock is wired (seed_check_phase=before): a difference is a founder
    write since the seed; the run stops before any subscription exists;
  - after (seed_check_phase=after): it first waits for each subscription from
    the founder to confirm the founder's current WAL position (bounded), then
    compares; a difference that clears on a retry was in flight, one that
    stays is a lost write.

It prints table names and row counts only, never a row or a checksum (the seed
tables hold identities and credentials), and reaches each Postgres through
`docker exec` on the host (the local socket), never with a password on a
command line.

The play runs here through a real ansible-playbook, the founder and the joiner
both on this machine with no privilege, against a stand-in `docker` that
answers the play's queries from files: a table's `<side>/<db>.<table>` holds
its rows, one per line.
"""
import json
import os
import shutil
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
PLAY = ROOT / "ops/ansible/spock-seed-current-check.yml"
SEED_PLAY = ROOT / "ops/ansible/site-seed-from-peer.yml"
CODE = "GB-367"

# The stand-in docker: `docker exec [-i] <container> psql ... -d <db> ... -c|-tAc <sql>`.
# The container names the side (pg-founder / pg-joiner). It records every argument list.
STAND_IN = r'''#!/usr/bin/env python3
import hashlib, json, os, re, sys
d = os.environ["SEED_STANDIN"]
open(os.path.join(d, "argv.log"), "a").write(json.dumps(sys.argv[1:]) + "\n")
a = sys.argv[1:]
if not a or a[0] != "exec":
    sys.exit(2)
a = [x for x in a[1:] if x != "-i"]
side = a[0].replace("pg-", "")
db = a[a.index("-d") + 1]
sql = a[-1]
state = json.load(open(os.path.join(d, "state.json")))
def rows(tab):
    p = os.path.join(d, side, f"{db}.{tab}")
    return open(p).read().split("\n")[:-1] if os.path.exists(p) else []
if "seed-check-sums" in sql:
    for tab in re.findall(r"'([a-z_]+)' AS t", sql):
        r = rows(tab)
        h = hashlib.md5("".join(sorted(hashlib.md5(x.encode()).hexdigest() for x in r)).encode()).hexdigest()
        print(f"{tab}|{len(r)}|{h}")
    # A write in flight: it reaches the joiner just after the joiner's first comparison.
    if side == "joiner" and state.get("lag_once"):
        for tab, line in list(state["lag_once"].items()):
            p = os.path.join(d, "joiner", f"{db}.{tab}")
            if os.path.exists(p):
                open(p, "a").write(line + "\n")
                del state["lag_once"][tab]
        json.dump(state, open(os.path.join(d, "state.json"), "w"))
    sys.exit(0)
if "local_node" in sql:
    print("founder_node")
    sys.exit(0)
if "sub_show_status" in sql:
    # A query for replicating subscriptions only (the before-phase probe of a
    # resume) is answered for the databases the state says replicate.
    if "status = 'replicating'" not in sql or db in state.get("replicating", []):
        print(f"spk_{db}_founder_node_sub_founder_node_to_joiner_node")
    sys.exit(0)
if "wait_slot_confirm_lsn" in sql:
    sys.exit(3 if state.get("never_catches_up") else 0)
sys.exit(2)
'''


class SeedCurrentCheck(unittest.TestCase):
    def test_its_tables_are_the_seed_plays(self):
        """The check compares exactly the tables the seed copies."""
        seed = yaml.safe_load(SEED_PLAY.read_text())[0]["vars"]["seed_databases"]
        ours = yaml.safe_load(PLAY.read_text())[0]["vars"]["seed_databases"]
        self.assertEqual(ours, seed)

    def test_a_column_stamped_on_use_is_left_out_of_the_checksum(self):
        """NetBird stamps a token's last_used on every API call, and the join calls
        the API between the seed and the check: that column is not a founder write."""
        vars_ = yaml.safe_load(PLAY.read_text())[0]["vars"]
        self.assertEqual(vars_["seed_check_usage_columns"], {"personal_access_tokens": ["last_used"]})
        compare = (ROOT / "ops/ansible/tasks/spock-seed-compare.yml").read_text()
        self.assertEqual(compare.count("(to_jsonb(x){% for c in seed_check_usage_columns[t] %} - '{{ c }}'{% endfor %})::text"), 4,
                         "both sides, in the checksum and in its ordering")

    @unittest.skipUnless(shutil.which("ansible-playbook") or os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1",
                         "ansible-playbook is not installed (CI sets GOAT_TEST_REQUIRE_ANSIBLE=1)")
    def test_the_check_before_and_after_spock(self):
        tables = {"netbird": ["accounts", "users"], "kratos": ["identities", "identity_credentials"], "hydra": ["hydra_client"]}
        secret = "s3cret-credential-row"
        base = {"accounts": ["acc_1|zagros"], "users": ["u1|admin"], "identities": ["id1|me@example.org"],
                "identity_credentials": [f"c1|{secret}"], "hydra_client": ["cl1|portal"]}

        def run(phase, founder, joiner, state=None):
            with tempfile.TemporaryDirectory() as t:
                t = Path(t)
                for side, data in (("founder", founder), ("joiner", joiner)):
                    (t / side).mkdir()
                    for db, tabs in tables.items():
                        for tab in tabs:
                            (t / side / f"{db}.{tab}").write_text("".join(r + "\n" for r in data.get(tab, [])))
                (t / "state.json").write_text(json.dumps(state or {}))
                (t / "bin").mkdir()
                (t / "bin/docker").write_text(STAND_IN)
                (t / "bin/docker").chmod(0o755)
                inv = t / "hosts.ini"
                inv.write_text("[gamma_hosts]\njoiner ansible_connection=local seed_postgres_container=pg-joiner\n"
                               "[zagros_hosts]\nfounder ansible_connection=local seed_postgres_container=pg-founder\n")
                gv = t / "group_vars/gamma_hosts"
                gv.mkdir(parents=True)
                (gv / "vars.yml").write_text("join_founder_site: zagros\n")
                env = {k: v for k, v in os.environ.items() if not k.startswith("ANSIBLE_")}
                env.update({"PATH": f"{t / 'bin'}:{env['PATH']}", "SEED_STANDIN": str(t),
                            "ANSIBLE_BECOME": "False", "ANSIBLE_CONFIG": str(t / "none.cfg"),
                            "ANSIBLE_LOCAL_TEMP": str(t / "tmp"), "ANSIBLE_HOME": str(t / "home"),
                            "ANSIBLE_NOCOLOR": "1", "ANSIBLE_LOG_PATH": str(t / "ansible.log")})
                # The play, with the seed's table set narrowed to the stand-in's, from this directory.
                doc = yaml.safe_load(PLAY.read_text())
                doc[0]["become"] = False
                doc[0].pop("vars_files", None)
                doc[0]["vars"]["seed_databases"] = [{"db": db, "tables": tabs} for db, tabs in tables.items()]
                doc[0]["vars"]["seed_check_retry_delay"] = 0
                doc[0]["vars"]["seed_check_attempts"] = 3
                for task in doc[0]["tasks"]:
                    if "ansible.builtin.include_tasks" in task:
                        task["ansible.builtin.include_tasks"] = str(ROOT / "ops/ansible" / task["ansible.builtin.include_tasks"])
                (t / "play.yml").write_text(yaml.safe_dump(doc))
                r = subprocess.run(["ansible-playbook", "-i", str(inv), str(t / "play.yml"),
                                    "-e", "site_id=gamma", "-e", f"seed_check_phase={phase}"],
                                   cwd=t, env=env, capture_output=True, text=True, timeout=300)
                printed = r.stdout + r.stderr + ((t / "ansible.log").read_text() if (t / "ansible.log").exists() else "")
                argv = (t / "argv.log").read_text() if (t / "argv.log").exists() else ""
                return r.returncode, printed, argv

        changed = dict(base, identities=base["identities"] + ["id2|new@example.org"])
        cases = [
            ("before", "equal: passes", base, base, None, 0),
            ("before", "a founder write since the seed: refused", changed, base, None, 2),
            ("after", "equal once caught up: passes", base, base, None, 0),
            ("after", "a write in flight that arrives: passes on a retry", changed, base,
             {"lag_once": {"identities": "id2|new@example.org"}}, 0),
            ("after", "a lost write that never arrives: refused", changed, base, None, 2),
            ("after", "a subscription that never catches up: refused", base, base, {"never_catches_up": True}, 2),
            # A resumed join-node (--from 13) on a joiner pass 3 already wired runs the
            # before check. A database replicating from the founder is past its seed and
            # read as after; one with no subscription must still equal the founder.
            ("before", "resume: a replicating database's write in flight arrives: passes", changed, base,
             {"replicating": ["kratos"], "lag_once": {"identities": "id2|new@example.org"}}, 0),
            ("before", "resume: every database replicating and equal: passes", base, base,
             {"replicating": ["netbird", "kratos", "hydra"]}, 0),
            ("before", "resume: a database not yet subscribed differs since the seed: refused", changed, base,
             {"replicating": ["netbird", "hydra"]}, 2),
        ]
        for phase, what, founder, joiner, state, want in cases:
            with self.subTest(phase=phase, case=what):
                rc, printed, argv = run(phase, founder, joiner, state)
                if want == 0:
                    self.assertEqual(rc, 0, printed[-3000:])
                    self.assertNotIn(CODE + ":", printed)
                else:
                    self.assertNotEqual(rc, 0, printed[-3000:])
                    self.assertIn(CODE + ":", printed, printed[-3000:])
                if "lost write" in what or "since the seed" in what or "not yet subscribed" in what:
                    # Names the table and its two counts, and nothing of the rows.
                    self.assertIn("identities", printed)
                    self.assertRegex(printed, r"identities: founder 2 rows, joiner 1")
                if "not yet subscribed" in what:
                    self.assertIn("Re-seed first", printed, printed[-3000:])
                if "since the seed" in what and "resume" not in what:
                    self.assertIn("Re-seed first", printed, printed[-3000:])
                if "never catches up" in what:
                    self.assertIn("did not catch up", printed)
                for leak in (secret, "me@example.org", "new@example.org"):
                    self.assertNotIn(leak, printed, f"{what}: a row value is printed")
                self.assertNotRegex(printed, r"\b[0-9a-f]{32}\b", f"{what}: a checksum is printed")
                self.assertNotRegex(argv.lower(), r"password|pgpassword|\bpass=", f"{what}: a password on a command line")


if __name__ == "__main__":
    unittest.main()
