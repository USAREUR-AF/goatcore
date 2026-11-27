#!/usr/bin/env python3
"""After a fresh standup the founder's own peer is in its site's INFRA group, so
join-node phase 10 has groups to give a new node.

Runs ops/ansible/netbird-infra-peers.yml against a stand-in NetBird management
API on localhost that keeps its peers and groups in memory.
"""
import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import sys
import threading
import unittest

ROOT = Path(__file__).resolve().parents[1]
PLAY = ROOT / "ops/ansible/netbird-infra-peers.yml"
TOKEN = "fixture-token-not-a-secret"


def require_ansible_playbook(case):
    """Skip without ansible-playbook, unless GOAT_TEST_REQUIRE_ANSIBLE=1 (CI sets
    it), where its absence fails the test instead of skipping it quietly."""
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


class FakeNetBird:
    def __init__(self, peers, groups, token=TOKEN, changed_meanwhile=None):
        self.peers, self.groups, self.puts = peers, groups, []
        self.token, self.changed_meanwhile = token, changed_meanwhile or {}
        outer = self

        class H(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def reply(self, code, body):
                data = json.dumps(body).encode()
                self.send_response(code)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def authorised(self):
                if self.headers.get("Authorization") != f"Token {outer.token}":
                    self.reply(401, {"message": "unauthorised"})
                    return False
                return True

            def do_GET(self):
                if not self.authorised():
                    return
                if self.path == "/api/peers":
                    self.reply(200, outer.peers)
                elif self.path == "/api/groups":
                    self.reply(200, outer.groups)
                elif self.path.startswith("/api/groups/"):
                    gid = self.path.rsplit("/", 1)[-1]
                    g = next((g for g in outer.groups if g["id"] == gid), None)
                    if g is None:
                        self.reply(404, {})
                        return
                    # Another writer's change between the play's two reads.
                    for p in outer.changed_meanwhile.get(gid, []):
                        if p not in [x["id"] for x in g["peers"]]:
                            g["peers"].append({"id": p, "name": p})
                    self.reply(200, g)
                else:
                    self.reply(404, {})

            def do_PUT(self):
                if not self.authorised():
                    return
                gid = self.path.rsplit("/", 1)[-1]
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                outer.puts.append((gid, body))
                for g in outer.groups:
                    if g["id"] == gid:
                        g["peers"] = [{"id": p, "name": p} for p in body["peers"]]
                        self.reply(200, g)
                        return
                self.reply(404, {})

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"

    def close(self):
        self.server.shutdown()


def peer(pid, name, ip, hostname=None, user_id=""):
    return {"id": pid, "name": name, "hostname": hostname or name, "ip": ip, "user_id": user_id}


def group(gid, name, peers=(), resources=()):
    return {"id": gid, "name": name, "peers": [{"id": p, "name": p} for p in peers], "resources": list(resources)}


class InfraPeers(unittest.TestCase):
    def setUp(self):
        require_ansible_playbook(self)
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def run_play(self, api, hosts=None, extra=(), raw_lines=None, no_site_group=False):
        """`hosts` maps each inventory host to the mesh address its own
        `netbird status` reports, or None for a host that is not on the mesh."""
        hosts = hosts if hosts is not None else {"mgmt-zagros-01": "100.127.0.10"}
        bindir = self.dir / "bin"
        bindir.mkdir(exist_ok=True)
        lines = []
        for h, ip in hosts.items():
            stub = bindir / f"netbird-{h}"
            stub.write_text("#!/bin/sh\n" + (f'echo \'{{"netbirdIp": "{ip}/16", "management": {{"connected": true}}}}\'' if ip else 'echo "Daemon status: NeedsLogin" >&2; exit 1') + "\n")
            stub.chmod(0o755)
            # Each host's python sets which stand-in its `netbird` runs.
            py = bindir / f"python-{h}"
            py.write_text(f'#!/bin/sh\nNETBIRD_STUB={stub} exec {sys.executable} "$@"\n')
            py.chmod(0o755)
            lines.append(f"{h} ansible_connection=local ansible_python_interpreter={py}")
        # The play runs `netbird status --json` on each host; here `netbird` is
        # that host's stand-in.
        # With no host's stand-in chosen, `netbird` is this machine's own: it
        # reports the operator machine's peer, so a read made here would find it.
        (bindir / "netbird").write_text('#!/bin/sh\n[ -n "$NETBIRD_STUB" ] && exec "$NETBIRD_STUB" "$@"\n'
                                        'echo \'{"netbirdIp": "100.127.0.50/16", "management": {"connected": true}}\'\n')
        (bindir / "netbird").chmod(0o755)
        inv = self.dir / "inv.ini"
        inv.write_text("[zagros_hosts]\n" + "".join(l + "\n" for l in (raw_lines if raw_lines is not None else lines)))
        env = dict(os.environ, ANSIBLE_NOCOLOR="1", ANSIBLE_RETRY_FILES_ENABLED="0", ANSIBLE_TIMEOUT="3", ANSIBLE_HOST_KEY_CHECKING="False",
                   ZAGROS_NB_API_TOKEN=TOKEN, PATH=f"{bindir}:{os.environ['PATH']}")
        return subprocess.run(
            ["ansible-playbook", "-i", "localhost,", "-i", str(inv), str(PLAY),
             "-e", "target_site=zagros", *([] if no_site_group else ["-e", "site_group=zagros_hosts"]), "-e", f"netbird_mgmt_url={api.url}",
             "-e", "netbird_infra_status_become=false",
             *extra],
            capture_output=True, text=True, env=env, cwd=self.dir, timeout=180)

    def test_a_fresh_site_puts_its_founder_in_infra(self):
        api = FakeNetBird([peer("p1", "mgmt-zagros-01", "100.127.0.10"), peer("p9", "a-laptop", "100.127.0.99", user_id="u1")],
                          [group("gA", "All", ["p1", "p9"]), group("gI", "ZAGROS-INFRA", resources=[{"id": "r1", "type": "host"}])])
        try:
            r = self.run_play(api, extra=["-vvv"])
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertEqual(api.puts, [("gI", {"name": "ZAGROS-INFRA", "peers": ["p1"], "resources": [{"id": "r1", "type": "host"}]})])
            self.assertIn("1 peer(s) added", r.stdout)
            self.assertNotIn(TOKEN, r.stdout + r.stderr)
        finally:
            api.close()

    def test_a_peer_is_matched_by_the_hosts_own_address_not_by_its_name(self):
        # A peer may call itself anything; the one at the host's address is the host.
        api = FakeNetBird([peer("pX", "mgmt-zagros-01", "100.127.0.77"), peer("p1", "box", "100.127.0.10", hostname="box")],
                          [group("gI", "ZAGROS-INFRA")])
        try:
            r = self.run_play(api)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertEqual(api.puts[0][1]["peers"], ["p1"])
        finally:
            api.close()

    def test_a_peer_squatting_the_name_of_a_host_off_the_mesh_gets_nothing(self):
        api = FakeNetBird([peer("pX", "mgmt-zagros-01", "100.127.0.77", user_id="u1")], [group("gI", "ZAGROS-INFRA")])
        try:
            r = self.run_play(api, hosts={"mgmt-zagros-01": None})
            self.assertNotEqual(r.returncode, 0, r.stdout)
            self.assertIn("not on the mesh", r.stdout)
            self.assertEqual(api.puts, [])
        finally:
            api.close()

    def test_a_peer_a_person_enrolled_at_the_address_gets_nothing(self):
        api = FakeNetBird([peer("p1", "mgmt-zagros-01", "100.127.0.10", user_id="u1")], [group("gI", "ZAGROS-INFRA")])
        try:
            r = self.run_play(api)
            self.assertNotEqual(r.returncode, 0, r.stdout)
            self.assertIn("enrolled by a person", r.stdout)
            self.assertEqual(api.puts, [])
        finally:
            api.close()

    def test_an_address_no_peer_or_two_peers_hold_writes_nothing(self):
        for peers, says in (([peer("p9", "x", "100.127.0.99")], "no peer holds its address"),
                            ([peer("p1", "a", "100.127.0.10"), peer("p2", "b", "100.127.0.10")], "2 peers hold its address")):
            with self.subTest(says):
                api = FakeNetBird(peers, [group("gI", "ZAGROS-INFRA")])
                try:
                    r = self.run_play(api)
                    self.assertNotEqual(r.returncode, 0, r.stdout)
                    self.assertIn(says, r.stdout)
                    self.assertEqual(api.puts, [])
                finally:
                    api.close()

    def test_a_peer_already_there_changes_nothing_and_others_stay(self):
        api = FakeNetBird([peer("p1", "mgmt-zagros-01", "100.127.0.10"), peer("p2", "mgmt-zagros-02", "100.127.0.11")],
                          [group("gI", "ZAGROS-INFRA", ["p7", "p1"])])
        try:
            r = self.run_play(api)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertEqual(api.puts, [])
            self.assertIn("nothing changed", r.stdout)
            r = self.run_play(api, hosts={"mgmt-zagros-01": "100.127.0.10", "mgmt-zagros-02": "100.127.0.11"})
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertEqual(api.puts, [("gI", {"name": "ZAGROS-INFRA", "peers": ["p7", "p1", "p2"], "resources": []})])
        finally:
            api.close()

    def test_a_change_made_between_the_reads_is_kept(self):
        api = FakeNetBird([peer("p1", "mgmt-zagros-01", "100.127.0.10")], [group("gI", "ZAGROS-INFRA", ["p7"])], changed_meanwhile={"gI": ["pZ"]})
        try:
            r = self.run_play(api)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertEqual(api.puts[0][1]["peers"], ["p7", "pZ", "p1"])
        finally:
            api.close()

    def test_a_host_is_read_over_its_own_transport_never_on_this_machine(self):
        # A host with only an address: its status is read over ssh, so this
        # machine's own netbird (which reports a key-enrolled peer) is never asked.
        api = FakeNetBird([peer("pL", "operator-laptop", "100.127.0.50")], [group("gI", "ZAGROS-INFRA")])
        try:
            r = self.run_play(api, hosts={"controller": "100.127.0.50"},
                              raw_lines=["mgmt-zagros-01 ansible_host=192.0.2.1 ansible_user=nobody",
                                         "mgmt-zagros-02 ansible_host=192.0.2.2 ansible_user=nobody"])
            self.assertNotEqual(r.returncode, 0, r.stdout)
            self.assertEqual(api.puts, [])
            self.assertIn("not on the mesh", r.stdout)
        finally:
            api.close()

    def test_two_hosts_at_one_address_write_nothing(self):
        api = FakeNetBird([peer("p1", "mgmt-zagros-01", "100.127.0.10")], [group("gI", "ZAGROS-INFRA")])
        try:
            r = self.run_play(api, hosts={"mgmt-zagros-01": "100.127.0.10", "mgmt-zagros-02": "100.127.0.10"})
            self.assertNotEqual(r.returncode, 0, r.stdout)
            self.assertIn("report the same mesh address", r.stdout)
            self.assertIn("mgmt-zagros-01 (100.127.0.10) mgmt-zagros-02 (100.127.0.10).", r.stdout)
            self.assertEqual(api.puts, [])
        finally:
            api.close()

    def test_check_mode_writes_nothing_and_says_so(self):
        api = FakeNetBird([peer("p1", "mgmt-zagros-01", "100.127.0.10")], [group("gI", "ZAGROS-INFRA")])
        try:
            r = self.run_play(api, extra=["--check"])
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertEqual(api.puts, [])
            self.assertIn("would be added (check mode)", r.stdout)
        finally:
            api.close()

    def test_a_refused_token_says_so_and_is_not_printed(self):
        api = FakeNetBird([peer("p1", "mgmt-zagros-01", "100.127.0.10")], [group("gI", "ZAGROS-INFRA")], token="another-token")
        try:
            r = self.run_play(api, extra=["-vvv"])
            self.assertNotEqual(r.returncode, 0, r.stdout)
            self.assertIn("is not accepted", r.stdout)
            self.assertNotIn(TOKEN, r.stdout + r.stderr)
            self.assertEqual(api.puts, [])
        finally:
            api.close()

    def test_no_infra_group_writes_nothing(self):
        api = FakeNetBird([peer("p1", "mgmt-zagros-01", "100.127.0.10")], [group("gA", "All", ["p1"])])
        try:
            r = self.run_play(api)
            self.assertNotEqual(r.returncode, 0, r.stdout)
            self.assertIn("ZAGROS-INFRA is missing", r.stdout)
            self.assertEqual(api.puts, [])
        finally:
            api.close()


    def test_no_site_group_is_refused_not_a_silent_no_op(self):
        # The first play's hosts default to a group no inventory holds, so
        # without -e site_group it reads nothing; the second play must refuse.
        api = FakeNetBird([peer("p1", "mgmt-zagros-01", "100.127.0.10")], [group("gI", "ZAGROS-INFRA")])
        try:
            r = self.run_play(api, no_site_group=True)
            self.assertNotEqual(r.returncode, 0, r.stdout)
            self.assertIn("site_group", r.stdout + r.stderr)
            self.assertEqual(api.puts, [])
        finally:
            api.close()


if __name__ == "__main__":
    unittest.main()
