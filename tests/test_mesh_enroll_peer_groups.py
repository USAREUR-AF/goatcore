"""mesh-enroll-host.yml reads a live peer's groups with the query NetBird's schema answers.

When join-node enrols a node (phase 10), mesh-enroll-host.yml mints the setup key
with the groups the minting host's own peer holds. It reads them with one psql
query on the NetBird database: the rows of group_peers for the peer whose `ip`
is the minting host's mesh address, without the All group.

NetBird stores a peer's `ip` with gorm's JSON serializer, so the column holds the
address in double quotes ("100.127.0.3"), and the query compares against that.

- test_the_query_compares_the_ip_as_netbird_stores_it always runs: it renders the
  play's own query and checks its shape.
- test_the_query_against_a_live_netbird_schema runs only when
  ops/netbird/tests/peer-groups-query.sh has started the pinned NetBird
  management on a disposable Postgres and enrolled two real clients. It runs
  the play's query there and checks the groups it returns.

Run: python3 -m pytest -q tests/test_mesh_enroll_peer_groups.py
     bash ops/netbird/tests/peer-groups-query.sh   (the live case; see its header)
"""
import os
import shlex
import subprocess
import unittest

import jinja2
import yaml

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PLAY = os.path.join(REPO, "ops/ansible/mesh-enroll-host.yml")
TASK = "mesh-enroll | read the groups off that peer"


def walk(tasks):
    for t in tasks or []:
        yield t
        for key in ("block", "rescue", "always", "tasks"):
            yield from walk(t.get(key))


def query_template():
    with open(PLAY, encoding="utf-8") as fh:
        plays = yaml.safe_load(fh)
    for play in plays:
        for t in walk(play.get("tasks")):
            if t.get("name") == TASK:
                return t["ansible.builtin.command"]["argv"][-1]
    raise AssertionError(f"no task named {TASK!r} in {PLAY}")


def render(account, ip):
    env = jinja2.Environment(autoescape=jinja2.select_autoescape(default_for_string=False))
    return env.from_string(query_template()).render(
        _me_accounts={"stdout_lines": [account]}, _me_mint_ip=ip
    )


class PeerGroupsQuery(unittest.TestCase):
    def test_the_query_compares_the_ip_as_netbird_stores_it(self):
        sql = render("acc_zagros", "100.127.0.3")
        self.assertIn("""p.ip = '"100.127.0.3"'""", sql)
        self.assertIn("p.account_id = 'acc_zagros'", sql)
        self.assertIn("g.name <> 'All'", sql)
        self.assertIn("ORDER BY gp.group_id", sql)

    @unittest.skipUnless(
        os.environ.get("PEER_GROUPS_PSQL"),
        "no live NetBird schema: run ops/netbird/tests/peer-groups-query.sh",
    )
    def test_the_query_against_a_live_netbird_schema(self):
        psql = shlex.split(os.environ["PEER_GROUPS_PSQL"])
        account = os.environ["PEER_GROUPS_ACCOUNT"]

        def groups(ip):
            out = subprocess.run(
                psql + ["-tAc", render(account, ip)], capture_output=True, text=True, check=True
            )
            return [line for line in out.stdout.splitlines() if line.strip()]

        # The minting host's peer: its two groups, in order, and not All.
        self.assertEqual(groups(os.environ["PEER_GROUPS_PEER_IP"]), os.environ["PEER_GROUPS_WANT"].split(","))
        # Another peer's groups are its own.
        self.assertEqual(groups(os.environ["PEER_GROUPS_OTHER_IP"]), os.environ["PEER_GROUPS_OTHER_WANT"].split(","))
        # An address no peer holds: no group, so the play refuses before minting.
        self.assertEqual(groups("100.127.255.254"), [])


if __name__ == "__main__":
    unittest.main()
