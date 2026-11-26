#!/usr/bin/env python3
"""ops/validation/identity-roundtrip.sh against two stand-in Kratos admin APIs (one
for the founder, one for the joiner, replicating to each other in memory), for the
cases the partition harness cannot stage: an address that exists only at the
joiner, a create whose reply is lost after it commits, an admin API that never
answers, an address that is not URL-safe, and a signal while the probe waits.
The harness scenario kratos-roundtrip-probe-cleans-up covers the real Kratos."""
import json
import os
import re
import signal
import socket
import subprocess
import threading
import time
import unittest
import urllib.parse
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

PROBE = Path(__file__).resolve().parents[1] / "ops/validation/identity-roundtrip.sh"


class Store:
    """Identities shared by both stand-ins when replication is on."""

    def __init__(self):
        self.lock = threading.Lock()
        self.sites = {"founder": {}, "joiner": {}}
        self.replicate = True
        self.queries = []

    def put(self, site, ident):
        with self.lock:
            self.sites[site][ident["id"]] = ident
            if self.replicate:
                for other in self.sites:
                    self.sites[other][ident["id"]] = ident

    def delete(self, site, iid):
        with self.lock:
            found = self.sites[site].pop(iid, None) is not None
            if found and self.replicate:
                for other in self.sites:
                    self.sites[other].pop(iid, None)
            return found


def stub(store, site, lose_create_reply=False):
    class H(BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def reply(self, code, body=None):
            data = json.dumps(body).encode() if body is not None else b""
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            u = urllib.parse.urlparse(self.path)
            if u.path == "/admin/identities":
                store.queries.append((site, u.query))
                q = urllib.parse.parse_qs(u.query)
                email = (q.get("credentials_identifier") or [""])[0]
                hits = [i for i in store.sites[site].values() if i["traits"]["email"] == email]
                return self.reply(200, hits)
            m = re.match(r"^/admin/identities/([0-9a-f-]+)$", u.path)
            if m and m.group(1) in store.sites[site]:
                return self.reply(200, store.sites[site][m.group(1)])
            return self.reply(404, {"error": {"code": 404}})

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            ident = {"id": str(uuid.uuid4()), "traits": body["traits"], "state": body.get("state")}
            store.put(site, ident)
            if lose_create_reply:
                return self.reply(502, {"error": "upstream reset"})
            return self.reply(201, ident)

        def do_DELETE(self):
            iid = self.path.rsplit("/", 1)[-1]
            return self.reply(204 if store.delete(site, iid) else 404)

    srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv, f"http://127.0.0.1:{srv.server_address[1]}"


def silent():
    """An admin API that accepts the connection and never answers."""
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    s.listen(16)
    return s, f"http://127.0.0.1:{s.getsockname()[1]}"


def run(*args, timeout=120):
    env = {"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "/tmp")}
    return subprocess.run(["bash", str(PROBE), *args], capture_output=True, text=True, env=env, timeout=timeout)


class Roundtrip(unittest.TestCase):
    def setUp(self):
        self.store = Store()
        self.servers = []

    def tearDown(self):
        for s in self.servers:
            if isinstance(s, socket.socket):
                s.close()
            else:
                s.shutdown()
                s.server_close()

    def pair(self, lose=False):
        f, fu = stub(self.store, "founder", lose_create_reply=lose)
        j, ju = stub(self.store, "joiner")
        self.servers += [f, j]
        return fu, ju

    def test_a_round_trip_passes_and_leaves_nothing(self):
        fu, ju = self.pair()
        r = run("--write", "--founder", fu, "--joiner", ju, "--timeout", "10")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("identity-roundtrip: PASS ", r.stdout)
        self.assertEqual(self.store.sites, {"founder": {}, "joiner": {}})

    def test_an_address_at_the_joiner_only_is_refused_and_kept(self):
        fu, ju = self.pair()
        self.store.replicate = False
        email = "goat-roundtrip-there@example.org"
        self.store.put("joiner", {"id": str(uuid.uuid4()), "traits": {"email": email}})
        r = run("--write", "--founder", fu, "--joiner", ju, "--email", email)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn("already exists at the joiner; nothing was written", r.stderr)
        self.assertEqual(len(self.store.sites["joiner"]), 1)
        self.assertEqual(self.store.sites["founder"], {})

    def test_a_create_whose_reply_is_lost_is_found_and_deleted(self):
        fu, ju = self.pair(lose=True)
        r = run("--write", "--founder", fu, "--joiner", ju, "--timeout", "10")
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("cleaned up: deleted", r.stdout)
        self.assertEqual(self.store.sites, {"founder": {}, "joiner": {}})

    def test_an_admin_api_that_never_answers_is_refused_in_bounded_time(self):
        fu, _ = self.pair()
        s, su = silent()
        self.servers.append(s)
        start = time.monotonic()
        r = run("--write", "--founder", fu, "--joiner", su, timeout=60)
        self.assertLess(time.monotonic() - start, 30)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn("nothing was written", r.stderr)
        self.assertEqual(self.store.sites["founder"], {})

    def test_an_address_is_url_encoded_in_the_lookup(self):
        fu, ju = self.pair()
        self.store.replicate = False
        email = "goat-roundtrip-a+b#c&d@example.org"
        self.store.put("founder", {"id": str(uuid.uuid4()), "traits": {"email": email}})
        r = run("--write", "--founder", fu, "--joiner", ju, "--email", email)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn("already exists at the founder", r.stderr)
        self.assertIn("credentials_identifier=" + urllib.parse.quote(email, safe=""), [q for _, q in self.store.queries][0])

    def test_a_signal_while_waiting_deletes_the_identity_and_exits_143(self):
        fu, ju = self.pair()
        self.store.replicate = False
        env = {"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "/tmp")}
        p = subprocess.Popen(["bash", str(PROBE), "--write", "--founder", fu, "--joiner", ju, "--timeout", "60"],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
        for _ in range(100):
            if self.store.sites["founder"]:
                break
            time.sleep(0.1)
        self.assertTrue(self.store.sites["founder"], "the probe created its identity")
        time.sleep(1)
        p.send_signal(signal.SIGTERM)
        out, err = p.communicate(timeout=30)
        self.assertEqual(p.returncode, 143, out + err)
        self.assertIn("cleaned up: deleted", out)
        self.assertEqual(self.store.sites["founder"], {})


if __name__ == "__main__":
    unittest.main()
