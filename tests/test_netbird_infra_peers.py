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
    def __init__(self, peers, groups):
        self.peers, self.groups, self.puts = peers, groups, []
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
                if self.headers.get("Authorization") != f"Token {TOKEN}":
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


def peer(pid, name, hostname=None):
    return {"id": pid, "name": name, "hostname": hostname or name, "ip": "100.127.0.10"}


def group(gid, name, peers=()):
    return {"id": gid, "name": name, "peers": [{"id": p, "name": p} for p in peers]}


class InfraPeers(unittest.TestCase):
    def setUp(self):
        require_ansible_playbook(self)
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def run_play(self, api, hosts=("mgmt-zagros-01",)):
        inv = self.dir / "inv.ini"
        inv.write_text("[zagros_hosts]\n" + "".join(f"{h}\n" for h in hosts))
        env = dict(os.environ, ANSIBLE_NOCOLOR="1", ANSIBLE_RETRY_FILES_ENABLED="0", ZAGROS_NB_API_TOKEN=TOKEN)
        return subprocess.run(
            ["ansible-playbook", "-i", "localhost,", "-i", str(inv), "--connection", "local", str(PLAY),
             "-e", "target_site=zagros", "-e", "site_group=zagros_hosts", "-e", f"netbird_mgmt_url={api.url}",
             "-e", "ansible_python_interpreter={{ ansible_playbook_python }}"],
            capture_output=True, text=True, env=env, cwd=self.dir, timeout=180)

    def test_a_fresh_site_puts_its_founder_in_infra(self):
        api = FakeNetBird([peer("p1", "mgmt-zagros-01"), peer("p9", "a-laptop")], [group("gA", "All", ["p1", "p9"]), group("gI", "ZAGROS-INFRA")])
        try:
            r = self.run_play(api)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertEqual(api.puts, [("gI", {"name": "ZAGROS-INFRA", "peers": ["p1"]})])
            self.assertIn("1 peer(s) added", r.stdout)
            self.assertNotIn(TOKEN, r.stdout + r.stderr)
        finally:
            api.close()

    def test_a_peer_already_there_changes_nothing_and_others_stay(self):
        api = FakeNetBird([peer("p1", "mgmt-zagros-01"), peer("p2", "mgmt-zagros-02")], [group("gI", "ZAGROS-INFRA", ["p7", "p1"])])
        try:
            r = self.run_play(api)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertEqual(api.puts, [])
            self.assertIn("nothing changed", r.stdout)
            r = self.run_play(api, hosts=("mgmt-zagros-01", "mgmt-zagros-02"))
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertEqual(api.puts, [("gI", {"name": "ZAGROS-INFRA", "peers": ["p7", "p1", "p2"]})])
        finally:
            api.close()

    def test_a_peer_matched_by_hostname(self):
        api = FakeNetBird([peer("p1", "box-renamed", hostname="mgmt-zagros-01")], [group("gI", "ZAGROS-INFRA")])
        try:
            r = self.run_play(api)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertEqual(api.puts[0][1]["peers"], ["p1"])
        finally:
            api.close()

    def test_a_host_with_no_peer_writes_nothing(self):
        api = FakeNetBird([peer("p9", "a-laptop")], [group("gI", "ZAGROS-INFRA")])
        try:
            r = self.run_play(api)
            self.assertNotEqual(r.returncode, 0, r.stdout)
            self.assertIn("no peer named so", r.stdout)
            self.assertEqual(api.puts, [])
        finally:
            api.close()

    def test_two_peers_of_one_name_write_nothing(self):
        api = FakeNetBird([peer("p1", "mgmt-zagros-01"), peer("p2", "mgmt-zagros-01")], [group("gI", "ZAGROS-INFRA")])
        try:
            r = self.run_play(api)
            self.assertNotEqual(r.returncode, 0, r.stdout)
            self.assertIn("2 peers named so", r.stdout)
            self.assertEqual(api.puts, [])
        finally:
            api.close()

    def test_no_infra_group_writes_nothing(self):
        api = FakeNetBird([peer("p1", "mgmt-zagros-01")], [group("gA", "All", ["p1"])])
        try:
            r = self.run_play(api)
            self.assertNotEqual(r.returncode, 0, r.stdout)
            self.assertIn("ZAGROS-INFRA is missing", r.stdout)
            self.assertEqual(api.puts, [])
        finally:
            api.close()


if __name__ == "__main__":
    unittest.main()
