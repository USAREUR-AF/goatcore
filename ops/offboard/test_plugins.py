#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""The shipped offboarding plug-ins against synthetic peers: a local HTTP
server standing in for the paging service and a script standing in for the
aws command. No real service or account is reached."""

import json
import os
import subprocess
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "scripts"))
import offboard_accounts  # noqa: E402


class Paging(BaseHTTPRequestHandler):
    users = {}          # id -> email
    refuse_delete = False
    seen_auth = []
    page_size = 100

    def log_message(self, *_args):
        pass

    def answer(self, code, value=None):
        body = b"" if value is None else json.dumps(value).encode()
        self.send_response(code)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        cls = type(self)
        cls.seen_auth.append(self.headers.get("Authorization"))
        if self.headers.get("Authorization") != "Token token=synthetic-key":
            return self.answer(401, {"error": {"message": "Unauthorized"}})
        query = parse_qs(urlparse(self.path).query)
        needle, offset = query["query"][0].casefold(), int(query.get("offset", ["0"])[0])
        # Like the real filter: a substring match, more than the exact login.
        rows = [{"id": uid, "email": email} for uid, email in sorted(cls.users.items())
                if needle in email.casefold() or needle.split("@")[0] in email.casefold()]
        page = rows[offset:offset + cls.page_size]
        self.answer(200, {"users": page, "more": offset + cls.page_size < len(rows)})

    def do_DELETE(self):
        cls = type(self)
        if cls.refuse_delete:
            return self.answer(400, {"error": {"message": "The user cannot be deleted",
                                               "errors": ["user is on 1 schedule"]}})
        cls.users.pop(self.path.rsplit("/", 1)[1], None)
        self.answer(204)


AWS = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
state = pathlib.Path(os.environ["FIXTURE_STATE"])
args = sys.argv[1:]
with (state / "aws.jsonl").open("a") as stream:
    stream.write(json.dumps(args) + "\n")
assert args[0] == "iam" and args[-2:] == ["--output", "json"], args
verb, user = args[1], args[args.index("--user-name") + 1]
path = state / f"iam-{user}.json"
def missing():
    print(f"An error occurred (NoSuchEntity) when calling the operation: {user} cannot be found", file=sys.stderr)
    sys.exit(254)
if os.environ.get("FIXTURE_AWS") == "denied":
    print("An error occurred (AccessDenied) when calling the operation", file=sys.stderr); sys.exit(254)
if not path.exists(): missing()
doc = json.loads(path.read_text())
if verb == "get-user": print(json.dumps({"User": {"UserName": user}}))
elif verb == "get-login-profile":
    if not doc["password"]: missing()
    print(json.dumps({"LoginProfile": {"UserName": user}}))
elif verb == "list-access-keys":
    print(json.dumps({"AccessKeyMetadata": [{"AccessKeyId": k, "Status": s} for k, s in doc["keys"].items()]}))
elif verb == "delete-login-profile":
    doc["password"] = False; path.write_text(json.dumps(doc))
elif verb == "update-access-key":
    key = args[args.index("--access-key-id") + 1]
    if os.environ.get("FIXTURE_AWS") != "sticky-key":
        doc["keys"][key] = args[args.index("--status") + 1]
    path.write_text(json.dumps(doc))
else:
    sys.exit(2)
'''


class PluginTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.state = Path(self.temp.name)
        self.repo = HERE.parent.parent

    def call(self, account, verb, who, env=None):
        merged = dict(os.environ, FIXTURE_STATE=str(self.state), **(env or {}))
        old = dict(os.environ)
        os.environ.clear()
        os.environ.update(merged)
        try:
            return offboard_accounts.call(self.repo, account, verb, who)
        finally:
            os.environ.clear()
            os.environ.update(old)

    # --- pagerduty

    def paging(self, users):
        handler = type("PagingPeer", (Paging,), {"users": dict(users), "refuse_delete": False,
                                                  "seen_auth": [], "page_size": 100})
        server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.shutdown)
        self.addCleanup(server.server_close)
        account = {"id": "paging", "type": "pagerduty", "match": "email",
                   "credential_env": ["PAGING_TOKEN"],
                   "options": {"api": f"http://127.0.0.1:{server.server_port}"}}
        return handler, account

    KEY = {"PAGING_TOKEN": "synthetic-key"}

    def test_pagerduty_matches_the_exact_login_only(self):
        handler, account = self.paging({"P1": "person@example.test",
                                        "P2": "person@example.test.invalid",
                                        "P3": "other.person@example.test"})
        answer = self.call(account, "check", ["Person@Example.test"], self.KEY)
        self.assertEqual(answer["items"], ["user P1"])
        answer = self.call(account, "remove", ["Person@Example.test"], self.KEY)
        self.assertEqual(answer, {"held": False, "items": [], "detail": "no user with this login"})
        self.assertEqual(sorted(handler.users), ["P2", "P3"])

    def test_pagerduty_reads_every_page(self):
        users = {f"X{n:03}": f"person+{n}@example.test" for n in range(7)}
        users["P9"] = "person@example.test"
        handler, account = self.paging(users)
        handler.page_size = 3
        self.assertEqual(self.call(account, "check", ["person@example.test"], self.KEY)["items"],
                         ["user P9"])

    def test_pagerduty_nothing_held_removes_nothing(self):
        handler, account = self.paging({"P3": "other@example.test"})
        self.assertFalse(self.call(account, "remove", ["person@example.test"], self.KEY)["held"])
        self.assertEqual(sorted(handler.users), ["P3"])

    def test_pagerduty_refused_delete_fails_with_the_service_reason(self):
        handler, account = self.paging({"P1": "person@example.test"})
        handler.refuse_delete = True
        with self.assertRaisesRegex(RuntimeError, "user P1 was not deleted, HTTP 400: .*on 1 schedule"):
            self.call(account, "remove", ["person@example.test"], self.KEY)

    def test_pagerduty_refused_credential_fails_and_is_never_printed(self):
        _, account = self.paging({"P1": "person@example.test"})
        with self.assertRaisesRegex(RuntimeError, "user list returned HTTP 401") as caught:
            self.call(account, "check", ["person@example.test"], {"PAGING_TOKEN": "wrong-key"})
        self.assertNotIn("wrong-key", str(caught.exception))

    # --- aws-iam

    def iam(self, users):
        script = self.state / "aws"
        script.write_text(AWS)
        script.chmod(0o755)
        for name, doc in users.items():
            (self.state / f"iam-{name}.json").write_text(json.dumps(doc))
        return {"id": "cloud", "type": "aws-iam", "match": "declared", "credential_env": [],
                "options": {"aws_command": [str(script)]}}

    def iam_calls(self):
        return [json.loads(line)[1] for line in (self.state / "aws.jsonl").read_text().splitlines()]

    def test_aws_iam_disables_password_and_active_keys_and_keeps_the_user(self):
        account = self.iam({"synthetic-user": {"password": True,
                                               "keys": {"AKIAFIRST": "Active", "AKIAOLD": "Inactive"}}})
        answer = self.call(account, "check", ["synthetic-user"])
        self.assertEqual(answer["items"], ["IAM user synthetic-user: console password",
                                           "IAM user synthetic-user: 1 active access key(s)"])
        self.assertNotIn("delete-login-profile", self.iam_calls())
        answer = self.call(account, "remove", ["synthetic-user"])
        self.assertFalse(answer["held"])
        doc = json.loads((self.state / "iam-synthetic-user.json").read_text())
        self.assertEqual(doc, {"password": False, "keys": {"AKIAFIRST": "Inactive", "AKIAOLD": "Inactive"}})
        self.assertNotIn("delete-user", self.iam_calls())

    def test_aws_iam_unknown_user_holds_nothing(self):
        account = self.iam({})
        self.assertEqual(self.call(account, "remove", ["no-such-user"])["held"], False)
        self.assertEqual(self.iam_calls(), ["get-user", "get-user"])

    def test_aws_iam_key_that_stays_active_is_reported_held(self):
        account = self.iam({"synthetic-user": {"password": False, "keys": {"AKIAFIRST": "Active"}}})
        answer = self.call(account, "remove", ["synthetic-user"], {"FIXTURE_AWS": "sticky-key"})
        self.assertEqual(answer["items"], ["IAM user synthetic-user: 1 active access key(s)"])

    def test_aws_iam_denied_call_fails_rather_than_reading_as_absent(self):
        account = self.iam({"synthetic-user": {"password": True, "keys": {}}})
        with self.assertRaisesRegex(RuntimeError, "iam get-user failed: .*AccessDenied"):
            self.call(account, "check", ["synthetic-user"], {"FIXTURE_AWS": "denied"})


if __name__ == "__main__":
    unittest.main()
