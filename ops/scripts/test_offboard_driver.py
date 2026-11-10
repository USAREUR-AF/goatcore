#!/usr/bin/env python3
"""End-to-end offboard driver tests with synthetic command and Kanidm peers."""

import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import threading
import unittest

import yaml

from offboard_driver import Kanidm, require_readable_status, ssh_revoke_evidence

HERE = Path(__file__).resolve().parent


class DoorHandler(BaseHTTPRequestHandler):
    expired = False
    deleted = False
    deletes = 0
    puts = 0

    def log_message(self, *_args):
        pass

    def answer(self, code, value, headers=None):
        body = json.dumps(value).encode()
        self.send_response(code)
        for key, val in (headers or {}).items():
            self.send_header(key, val)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        value = json.loads(self.rfile.read(length))
        if value == {"step": {"init": "idm_admin"}}:
            self.answer(200, {"state": "choose"}, {"x-kanidm-auth-session-id": "session"})
        elif value == {"step": {"begin": "password"}}:
            self.answer(200, {"state": "continue"}, {"x-kanidm-auth-session-id": "session"})
        else:
            self.answer(200, {"state": {"success": "door-token"}})

    def do_GET(self):
        if self.path != "/v1/person":
            self.answer(404, {})
            return
        if type(self).deleted:
            self.answer(200, [])
            return
        attrs = {"name": ["example-user"], "mail": ["person@example.test"]}
        if type(self).expired:
            attrs["account_expire"] = ["1970-01-01T00:00:00Z"]
        self.answer(200, [{"attrs": attrs}])

    def do_PUT(self):
        type(self).expired = True
        type(self).puts += 1
        self.answer(200, None)

    def do_DELETE(self):
        type(self).deleted = True
        type(self).deletes += 1
        self.answer(204, None)


class DriverTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        scripts = self.root / "ops/scripts"
        scripts.mkdir(parents=True)
        for name in ("offboard.sh", "offboard_driver.py", "offboard_subject.py"):
            shutil.copy(HERE / name, scripts)
        manifest = self.root / "ops/operators/test/example-user.yml"
        manifest.parent.mkdir(parents=True)
        manifest.write_text(yaml.safe_dump({"identity": {
            "handle": "example-user", "email": "person@example.test", "status": "active",
            "github": "example"},
            "org": {"short_tag": "example-user"}}))
        # What the checkout declares: the organisations a person is removed from,
        # and the name its secret references carry for the public door password.
        declared = self.root / "ops/ansible/group_vars/all.yml"
        declared.parent.mkdir(parents=True)
        declared.write_text('goat_offboard_github_orgs: "first-org,second-org"\n'
                            'public_door_idm_admin_pw_env: "DOOR_PW_UNDER_ANOTHER_NAME"  # declared\n')
        self.home = self.root / "home"
        self.home.mkdir()
        self.bindir = self.root / "bin"
        self.bindir.mkdir()
        self.state = self.root / "state"
        self.state.mkdir()
        stub = self.bindir / "stub"
        stub.write_text(STUB)
        stub.chmod(0o755)
        for name in ("git", "goat", "gh", "op", "sleep"):
            (self.bindir / name).symlink_to(stub)
        self.handlers = []
        self.servers = []
        for name in ("CollabDoor", "PublicDoor"):
            handler = type(name, (DoorHandler,), {"expired": False, "deleted": False,
                                                  "deletes": 0, "puts": 0})
            server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
            threading.Thread(target=server.serve_forever, daemon=True).start()
            self.handlers.append(handler)
            self.servers.append(server)
            self.addCleanup(server.shutdown)
            self.addCleanup(server.server_close)

    def invoke(self, mode="success", *, apply=True, explicit_receipt=True,
               goat_profile=None, github_user=None, github_unverified=False,
               goatnets=None, openbao_user=None, bao_addr=None, door_env=None):
        receipt = self.root / "receipt.json"
        collab_url = f"http://127.0.0.1:{self.servers[0].server_port}"
        public_url = f"http://127.0.0.1:{self.servers[1].server_port}"
        env = dict(os.environ)
        env.update(PATH=str(self.bindir) + os.pathsep + os.environ["PATH"],
                   HOME=str(self.home), FIXTURE_ROOT=str(self.root),
                   FIXTURE_STATE=str(self.state), FIXTURE_MODE=mode,
                   COLLAB_KANIDM_URL=collab_url, COLLAB_KANIDM_IDM_ADMIN_PW="test-password",
                   PUBLIC_DOOR_KANIDM_URL=public_url, GOAT_OFFBOARD_GITHUB_ORGS="")
        for name in ("GOAT_IDM_ADMIN_PW", "DOOR_PW_UNDER_ANOTHER_NAME"):
            env.pop(name, None)
        env.update({"DOOR_PW_UNDER_ANOTHER_NAME": "test-password"} if door_env is None else door_env)
        env.pop("GOAT_PROFILE_DIR", None)
        env.pop("BAO_ADDR", None)
        if bao_addr is not None:
            env["BAO_ADDR"] = bao_addr
        if goat_profile is not None:
            env["GOAT_PROFILE_DIR"] = str(goat_profile)
        argv = ["bash", str(self.root / "ops/scripts/offboard.sh"), "example-user"]
        for goatnet in goatnets or ["test"]:
            argv += ["--goatnet", goatnet]
        if apply:
            argv += ["--apply", "--operator", "test-operator"]
        if github_user is not None:
            argv += ["--github-user", github_user]
        if github_unverified:
            argv += ["--github-user-unverified"]
        if openbao_user is not None:
            argv += ["--openbao-user", openbao_user]
        if explicit_receipt:
            argv += ["--receipt", str(receipt)]
        proc = subprocess.run(
            argv, cwd=self.root, env=env,
            text=True, capture_output=True, timeout=20,
        )
        value = json.loads(receipt.read_text()) if receipt.exists() else None
        return proc, value

    def add_manifest(self, relative_path, status="active", legacy_vendor=False,
                     github="example", email=None):
        manifest = self.root / "ops/operators/test" / relative_path
        manifest.parent.mkdir(parents=True, exist_ok=True)
        value = {
            "identity": {"handle": manifest.stem,
                         "email": email or f"{manifest.stem}@example.test",
                         "status": status, "github": github},
            "org": {"short_tag": "example-user"},
        }
        if legacy_vendor:
            value["vendor"] = value.pop("org")
        if github is None:
            value["identity"].pop("github")
        manifest.write_text(yaml.safe_dump(value))

    def calls(self):
        path = self.state / "calls.jsonl"
        return [json.loads(line) for line in path.read_text().splitlines()]

    def remove_manifest_github(self):
        self.set_manifest_github(None)

    def set_manifest_github(self, github):
        path = self.root / "ops/operators/test/example-user.yml"
        value = yaml.safe_load(path.read_text())
        if github is None:
            value["identity"].pop("github")
        else:
            value["identity"]["github"] = github
        path.write_text(yaml.safe_dump(value))

    @staticmethod
    def mutations(calls):
        return [call for call in calls if (
            call[0] == "goat" and ("revoke" in call or "--revoke" in call)
        ) or call[:4] == ["op", "group", "user", "revoke"] or (
            call[0] == "gh" and "-X" in call
        )]

    def test_full_success_saves_complete_receipt(self):
        proc, receipt = self.invoke()
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertEqual(receipt["outcome"], "success")
        rows = receipt["goatnets"]["test"]
        self.assertEqual(receipt["github"], {"login": "example", "source": "manifest"})
        self.assertEqual(rows["github"]["login"], "example")
        self.assertEqual(rows["github"]["login_source"], "manifest")
        self.assertEqual(set(rows), set((
            "host-ssh-keys", "operator-keys", "netbird-devices", "netbird-setup-keys",
            "netbird-user", "fabric-slot", "sponsorship", "ory", "collab-kanidm",
            "public-door-kanidm", "onepassword", "github")))
        self.assertTrue(all(row["status"] == "removed-and-checked" for row in rows.values()))
        for row in rows.values():
            self.assertEqual(row["verified"], True)
            self.assertEqual(row["operator"], "test-operator")
            self.assertTrue(row["how"])
            self.assertTrue(row["service_identity"])
        self.assertTrue(any(row["before"] == "present" for row in rows.values()))
        self.assertEqual(self.handlers[0].deletes, 1)
        self.assertEqual(self.handlers[0].puts, 0)
        self.assertEqual(self.handlers[1].deletes, 0)
        self.assertEqual(self.handlers[1].puts, 1)
        self.assertTrue((self.state / "op-revoked-platform").exists())
        self.assertTrue((self.state / "op-revoked-software").exists())
        self.assertTrue((self.state / "gh-member-first-org").exists())
        self.assertTrue((self.state / "gh-outside-first-org").exists())
        self.assertIn("shared credentials to assess separately", proc.stdout)
        calls = self.calls()
        mutations = self.mutations(calls)
        self.assertEqual(mutations, [
            ["goat", "--context", "test-op", "admin", "platform-admin", "revoke",
             "person@example.test", "--env", "test", "--full", "--no-ory",
             "--no-1password", "--apply", "--json", "--github", "example"],
            ["goat", "--context", "test-op", "admin", "onboard", "example-user",
             "--env", "test", "--revoke", "--native", "--collab", "revoke"],
            ["op", "group", "user", "revoke", "--group", "platform",
             "--user", "person@example.test"],
            ["op", "group", "user", "revoke", "--group", "software",
             "--user", "person@example.test"],
            *(["gh", "api", "-X", "DELETE", f"orgs/{org}/{kind}/example"]
              for org in dict.fromkeys(("first-org", "second-org"))
              for kind in ("memberships", "outside_collaborators")),
        ])
        audits = [call for call in calls
                  if call[:4] == ["goat", "--context", "test-op", "audit"]]
        self.assertEqual([call[call.index("--action") + 1] for call in audits], [
            "offboard.host-ssh-keys", "offboard.operator-keys",
            "offboard.netbird-devices", "offboard.netbird-setup-keys",
            "offboard.netbird-user", "offboard.fabric-slot",
            "offboard.sponsorship", "offboard.ory", "offboard.collab-kanidm",
            "offboard.public-door-kanidm", "offboard.onepassword", "offboard.github",
        ])
        self.assertTrue(all(call[call.index("--actor") + 1] == "test-operator"
                            for call in audits))
        self.assertEqual(sum(call[:3] == ["op", "group", "list"] for call in calls), 2)

    def store(self, sessions, revoke_works=True):
        """A fake revoke-person: `sessions` live tokens until a real run ends them."""
        script = self.root / "ops/secrets/openbao/revoke-person"
        script.parent.mkdir(parents=True, exist_ok=True)
        (self.state / "bao-sessions").write_text(str(sessions))
        end = 'echo 0 > "$state"' if revoke_works else ":"
        script.write_text(f"""#!/usr/bin/env bash
state={self.state}/bao-sessions
echo "$*" >> {self.state}/bao-calls
n=$(cat "$state")
if [ "${{2:-}}" = --dry-run ]; then
  echo "revoke-person: $1: $n token(s) would be revoked across 1 entity(ies)"
else
  {end}
  echo "revoke-person: $1: $n token(s) revoked across 1 entity(ies)"
fi
""")
        script.chmod(0o755)

    def test_store_plane_ends_sessions_and_reads_back_zero(self):
        self.store(sessions=2)
        proc, receipt = self.invoke(openbao_user="example-user",
                                    bao_addr="https://store.example.test:8200")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        row = receipt["goatnets"]["test"]["openbao"]
        self.assertEqual((row["status"], row["before"], row["after"]),
                         ("removed-and-checked", "present", "absent"))
        self.assertEqual((self.state / "bao-calls").read_text().splitlines(), [
            "example-user --dry-run", "example-user", "example-user --dry-run"])
        audits = [call[call.index("--action") + 1] for call in self.calls()
                  if call[:4] == ["goat", "--context", "test-op", "audit"]]
        self.assertEqual(audits[-3:], ["offboard.onepassword", "offboard.openbao",
                                       "offboard.github"])

    def test_store_session_that_survives_fails_the_run(self):
        self.store(sessions=1, revoke_works=False)
        proc, receipt = self.invoke(openbao_user="example-user",
                                    bao_addr="https://store.example.test:8200")
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(receipt["outcome"], "failed")
        self.assertEqual(receipt["goatnets"]["test"]["openbao"]["status"], "check-failed")

    def test_store_address_without_store_login_refuses_before_mutation(self):
        self.store(sessions=1)
        proc, receipt = self.invoke(bao_addr="https://store.example.test:8200")
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("needs both BAO_ADDR and --openbao-user", proc.stderr)
        self.assertEqual(self.mutations(self.calls()), [])

    def test_no_store_means_no_store_plane(self):
        proc, receipt = self.invoke()
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertNotIn("openbao", receipt["goatnets"]["test"])

    def test_github_user_mismatch_refuses_before_every_mutation(self):
        proc, receipt = self.invoke(github_user="someone-else")
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(self.mutations(self.calls()), [])
        self.assertIn("does not match manifest identity.github", " ".join(receipt["errors"]))
        self.assertEqual(receipt["github"], {"login": "example", "source": "manifest"})
        self.assertEqual(receipt["goatnets"]["test"]["github"]["login_source"],
                         "manifest")

    def test_conflicting_manifest_github_logins_refuse_before_every_mutation(self):
        self.add_manifest("../test-two/example-user.yml", github="different",
                          email="person@example.test")
        proc, receipt = self.invoke(goatnets=["test", "test-two"])
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(self.mutations(self.calls()), [])
        self.assertIn("test=example", " ".join(receipt["errors"]))
        self.assertIn("test-two=different", " ".join(receipt["errors"]))
        self.assertEqual(receipt["github"]["source"], "unresolved")

    def test_missing_manifest_github_requires_explicit_acknowledgement(self):
        self.remove_manifest_github()
        proc, receipt = self.invoke(github_user="example")
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(self.mutations(self.calls()), [])
        self.assertIn("--github-user-unverified", " ".join(receipt["errors"]))
        self.assertEqual(receipt["github"]["source"], "unresolved")

    def test_subject_resolution_refusal_records_unresolved_github_source(self):
        self.set_manifest_github("@secret-login")
        proc, receipt = self.invoke()
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(self.mutations(self.calls()), [])
        self.assertEqual(receipt["github"], {"login": None, "source": "unresolved"})

    def test_acknowledged_unverified_github_login_is_recorded(self):
        self.remove_manifest_github()
        proc, receipt = self.invoke(github_user="example", github_unverified=True)
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(receipt["outcome"], "incomplete")
        self.assertEqual(receipt["github"],
                         {"login": "example", "source": "operator-unverified"})
        self.assertEqual(receipt["goatnets"]["test"]["github"]["login_source"],
                         "operator-unverified")
        self.assertEqual(receipt["goatnets"]["test"]["github"]["status"],
                         "removed-login-unbound")
        self.assertEqual(receipt["goatnets"]["test"]["host-ssh-keys"]["status"],
                         "removed-login-unbound")
        github_audit = next(call for call in self.calls()
                            if "offboard.github" in call)
        self.assertIn("github_source=operator-unverified",
                      github_audit[github_audit.index("--args-digest") + 1])

    def test_unverified_login_does_not_mask_failed_host_readback(self):
        self.remove_manifest_github()
        proc, receipt = self.invoke("host-unavailable", github_user="example",
                                    github_unverified=True)
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(receipt["outcome"], "failed")
        row = receipt["goatnets"]["test"]["host-ssh-keys"]
        self.assertEqual(row["status"], "check-failed")
        self.assertFalse(row["host_readback"])
        self.assertEqual(row["after"], "present-or-unknown")

    def test_unverified_nonexistent_github_login_fails_github_plane(self):
        self.remove_manifest_github()
        proc, receipt = self.invoke("nonexistent-github", github_user="missing",
                                    github_unverified=True)
        self.assertNotEqual(proc.returncode, 0)
        row = receipt["goatnets"]["test"]["github"]
        self.assertEqual(row["status"], "check-failed")
        self.assertEqual(row["login_source"], "operator-unverified")
        self.assertIn("does not exist", row["detail"])
        self.assertEqual(self.mutations(self.calls()), [])

    def test_github_login_cross_check_is_case_insensitive(self):
        proc, receipt = self.invoke(github_user="ExAmPlE")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertEqual(receipt["github"], {"login": "example", "source": "manifest"})

    def test_org_invitation_by_login_is_removed_and_read_back(self):
        proc, receipt = self.invoke("org-invitation")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertTrue((self.state / "gh-org-invite-first-org-101").exists())
        self.assertEqual(receipt["goatnets"]["test"]["github"]["before"], "present")

    def test_email_only_org_invitation_is_removed(self):
        proc, _ = self.invoke("email-invitation")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertTrue((self.state / "gh-org-invite-first-org-102").exists())

    def test_repository_invitation_is_removed(self):
        proc, _ = self.invoke("repo-invitation")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertTrue((self.state / "gh-repo-invite-first-org-goat-201").exists())

    def test_lingering_invitation_fails_and_names_receipt(self):
        proc, receipt = self.invoke("lingering-invitation")
        self.assertNotEqual(proc.returncode, 0)
        row = receipt["goatnets"]["test"]["github"]
        self.assertEqual(row["status"], "check-failed")
        self.assertIn("first-org", row["detail"])
        self.assertIn("invitation remains", row["detail"])

    def test_unreadable_invitation_list_fails_before_mutation(self):
        proc, receipt = self.invoke("unreadable-invitations")
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(self.mutations(self.calls()), [])
        self.assertIn("first-org organisation invitations", " ".join(receipt["errors"]))

    def test_public_door_password_is_read_from_the_neutral_name(self):
        proc, receipt = self.invoke(door_env={"GOAT_IDM_ADMIN_PW": "test-password"})
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertEqual(receipt["goatnets"]["test"]["public-door-kanidm"]["status"],
                         "removed-and-checked")

    def test_missing_public_door_password_names_the_neutral_variable(self):
        proc, receipt = self.invoke(door_env={})
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(self.mutations(self.calls()), [])
        self.assertIn("GOAT_IDM_ADMIN_PW is missing", " ".join(receipt["errors"]))

    def test_no_declared_github_organisation_fails_the_run(self):
        (self.root / "ops/ansible/group_vars/all.yml").write_text("")
        proc, receipt = self.invoke(door_env={"GOAT_IDM_ADMIN_PW": "test-password"})
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("GOAT_OFFBOARD_GITHUB_ORGS does not name any targets",
                      json.dumps(receipt))

    def test_skipped_system_fails_and_receipt_says_not_run(self):
        proc, receipt = self.invoke("skip")
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(receipt["outcome"], "failed")
        self.assertTrue(all(row["status"] == "not-run"
                            for row in receipt["goatnets"]["test"].values()))
        self.assertTrue(all(row["verified"] is False
                            for row in receipt["goatnets"]["test"].values()))
        self.assertTrue(all(row["operator"] == "test-operator"
                            for row in receipt["goatnets"]["test"].values()))
        self.assertFalse((self.state / "platform-mutated").exists())
        self.assertFalse((self.state / "onboard-mutated").exists())
        self.assertEqual(sum(handler.deletes + handler.puts for handler in self.handlers), 0)

    def test_lingering_access_fails_and_receipt_names_check(self):
        proc, receipt = self.invoke("lingering")
        self.assertNotEqual(proc.returncode, 0)
        row = receipt["goatnets"]["test"]["netbird-devices"]
        self.assertEqual(row["status"], "check-failed")
        self.assertEqual(row["after"], "present-or-unknown")

    def test_empty_console_without_host_readback_fails_ssh_plane(self):
        proc, receipt = self.invoke("host-unavailable")
        self.assertNotEqual(proc.returncode, 0)
        row = receipt["goatnets"]["test"]["host-ssh-keys"]
        self.assertEqual(row["status"], "check-failed")
        self.assertEqual(row["after"], "present-or-unknown")
        self.assertIn("host authorized_keys readback did not pass", row["detail"])

    def test_revoke_evidence_must_use_the_resolved_flag_login(self):
        for mode, diagnostic in (("wrong-github", "did not match"),
                                 ("proposal-github", "was not flag")):
            with self.subTest(mode=mode):
                proc, receipt = self.invoke(mode)
                self.assertNotEqual(proc.returncode, 0)
                row = receipt["goatnets"]["test"]["host-ssh-keys"]
                self.assertEqual(row["status"], "check-failed")
                self.assertIn(diagnostic, row["detail"])

    def test_manifest_bound_person_with_no_keys_is_not_applicable(self):
        proc, receipt = self.invoke("no-keys")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        row = receipt["goatnets"]["test"]["host-ssh-keys"]
        self.assertEqual(row["status"], "not-applicable-no-keys")
        self.assertTrue(row["verified"])
        self.assertEqual(row["github_login"], "example")
        self.assertEqual(row["console_rows_any_status"], 0)
        self.assertEqual(row["keys_checked"], 0)
        self.assertIn("readback_at", row)

    def test_no_key_result_requires_complete_evidence(self):
        for mode in ("github-fetch-failed", "no-keys-console-row",
                     "no-keys-boolean-count", "no-keys-float-count",
                     "no-keys-negative-count"):
            with self.subTest(mode=mode):
                proc, receipt = self.invoke(mode)
                self.assertNotEqual(proc.returncode, 0)
                self.assertNotEqual(
                    receipt["goatnets"]["test"]["host-ssh-keys"]["status"],
                    "not-applicable-no-keys")
        self.remove_manifest_github()
        proc, receipt = self.invoke("no-keys", github_user="example",
                                    github_unverified=True)
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(receipt["goatnets"]["test"]["host-ssh-keys"]["status"],
                         "removed-login-unbound")

    def test_preflight_console_key_cannot_become_not_applicable_without_host_check(self):
        proc, receipt = self.invoke("no-keys-pre-status-present")
        self.assertNotEqual(proc.returncode, 0)
        row = receipt["goatnets"]["test"]["host-ssh-keys"]
        self.assertEqual(row["status"], "check-failed")
        self.assertIn("pre-removal SSH status did not prove an empty console inventory",
                      row["detail"])

    def test_ssh_revoke_evidence_ignores_non_object_json(self):
        for line in ('42', '"x"', '[]', 'null', '{"direction":"revoke --full"}'):
            with self.subTest(line=line):
                proc = subprocess.CompletedProcess(["goat"], 0, line + "\n", "")
                self.assertIsNone(ssh_revoke_evidence(proc))

    def test_unreadable_check_fails_and_receipt_names_check(self):
        proc, receipt = self.invoke("unreadable")
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(receipt["outcome"], "failed")
        self.assertEqual(receipt["goatnets"]["test"]["ory"]["status"], "check-failed")
        for plane in ("host-ssh-keys", "operator-keys", "netbird-devices",
                      "netbird-setup-keys", "netbird-user", "fabric-slot",
                      "sponsorship", "ory"):
            self.assertEqual(receipt["goatnets"]["test"][plane]["status"], "check-failed")

    def test_driver_consumer_rejects_malformed_mesh_contract(self):
        base = {
            "read_errors": [], "operator-key": "none", "netbird-role": "user (blocked)",
            "ssh": "none", "sponsorship": "none", "ory_status": {"read_ok": True},
            "ssh_status": {"read_ok": True}, "setup_key_status": {"read_ok": True},
            "fabric_status": {"read_ok": True}, "sponsorship_status": {"read_ok": True},
        }
        for mesh in (None, {}, {"read_ok": False, "peer_ids": [], "errors": []},
                     {"read_ok": True, "peer_ids": [], "errors": ["failed"]},
                     {"read_ok": True, "peer_ids": "", "errors": []}):
            value = dict(base, mesh_status=mesh)
            with self.subTest(mesh=mesh), self.assertRaises(RuntimeError):
                require_readable_status(value, "status")

    def test_naive_or_malformed_kanidm_expiry_is_not_trusted(self):
        for value in ("1970-01-01T00:00:00", "not-a-timestamp", None):
            with self.subTest(value=value):
                attrs = {"account_expire": [value]} if value is not None else {}
                self.assertFalse(Kanidm.is_expired({"attrs": attrs}))

    def test_group_added_during_run_is_detected(self):
        proc, receipt = self.invoke("late-group")
        self.assertNotEqual(proc.returncode, 0)
        row = receipt["goatnets"]["test"]["onepassword"]
        self.assertEqual(row["status"], "check-failed")
        self.assertEqual(row["before"], "present")
        self.assertEqual(row["after"], "unknown")

    def test_default_receipt_is_private_and_outside_repository(self):
        proc, receipt = self.invoke(explicit_receipt=False)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIsNone(receipt)
        receipt_dir = self.home / ".goat/offboard"
        receipts = list(receipt_dir.glob("example-user-*.json"))
        self.assertEqual(len(receipts), 1)
        self.assertEqual(stat.S_IMODE(receipt_dir.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(receipts[0].stat().st_mode), 0o600)
        self.assertFalse((self.root / "ops/audit").exists())

    def test_goat_profile_dir_controls_default_receipt(self):
        profile = self.root / "private-profile"
        proc, receipt = self.invoke(explicit_receipt=False, goat_profile=profile)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIsNone(receipt)
        receipts = list((profile / "offboard").glob("example-user-*.json"))
        self.assertEqual(len(receipts), 1)
        self.assertEqual(stat.S_IMODE((profile / "offboard").stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(receipts[0].stat().st_mode), 0o600)

    def test_fabric_org_kept_for_active_teammate_passes_and_is_reported(self):
        self.add_manifest("teammate.yml", legacy_vendor=True)
        proc, receipt = self.invoke("fabric-kept")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        row = receipt["goatnets"]["test"]["fabric-slot"]
        self.assertEqual(row["status"], "removed-and-checked")
        self.assertEqual(row["after"], "retained-for-teammates")
        self.assertTrue(row["verified"])
        self.assertIn("1 other active member", row["detail"])
        self.assertIn({"credential": "shared org fabric certificate bundle",
                       "goatnet": "test", "org_tag": "example-user"},
                      receipt["shared_credentials"])
        self.assertIn("test org example-user fabric certificate bundle", proc.stdout)

    def test_fabric_org_remaining_without_teammates_fails(self):
        proc, receipt = self.invoke("fabric-kept")
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(receipt["goatnets"]["test"]["fabric-slot"]["status"],
                         "check-failed")
        self.assertEqual(len(receipt["shared_credentials"]), 2)

    def test_revoked_and_nested_same_tag_manifests_are_not_teammates(self):
        self.add_manifest("revoked-person.yml", status="revoked")
        self.add_manifest("nested/active-person.yml", status="active")
        proc, receipt = self.invoke("fabric-kept")
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(receipt["goatnets"]["test"]["fabric-slot"]["status"],
                         "check-failed")
        self.assertEqual(len(receipt["shared_credentials"]), 2)
        self.assertNotIn("shared org fabric certificate bundle",
                         [item["credential"] for item in receipt["shared_credentials"]])

    def test_help_documents_flags_planes_and_deliberate_exclusions(self):
        env = dict(os.environ, PATH=str(self.bindir) + os.pathsep + os.environ["PATH"],
                   HOME=str(self.home), FIXTURE_ROOT=str(self.root), FIXTURE_STATE=str(self.state))
        env.pop("GOAT_PROFILE_DIR", None)
        proc = subprocess.run(
            ["bash", str(self.root / "ops/scripts/offboard.sh"), "--help"],
            cwd=self.root, env=env, text=True, capture_output=True, timeout=20)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        for flag in ("--goatnet", "--all-goatnets", "--apply", "--github-user",
                     "--github-user-unverified",
                     "--operator", "--pubkey-file", "--receipt"):
            self.assertIn(flag, proc.stdout)
        for plane in ("host-ssh-keys", "operator-keys", "netbird-devices",
                      "netbird-setup-keys", "netbird-user", "fabric-slot",
                      "sponsorship", "ory", "collab-kanidm", "public-door-kanidm",
                      "onepassword", "github"):
            self.assertIn(plane, proc.stdout)
        help_text = " ".join(proc.stdout.split()).lower()
        for excluded in ("does not delete the Ory identity",
                         "does not touch machine custody",
                         "does not rotate shared credentials"):
            self.assertIn(excluded.lower(), help_text)

    def test_plan_lists_every_plane_and_creates_no_receipt(self):
        proc, receipt = self.invoke(apply=False, explicit_receipt=False)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIsNone(receipt)
        self.assertIn("GitHub login: example (manifest)", proc.stdout)
        for plane in ("host-ssh-keys", "operator-keys", "netbird-devices",
                      "netbird-setup-keys", "netbird-user", "fabric-slot",
                      "sponsorship", "ory", "collab-kanidm", "public-door-kanidm",
                      "onepassword", "github"):
            self.assertIn(plane, proc.stdout)
        self.assertFalse((self.home / ".goat/offboard").exists())

    def test_plan_warns_that_unverified_github_login_stays_incomplete(self):
        self.remove_manifest_github()
        proc, receipt = self.invoke(apply=False, explicit_receipt=False,
                                    github_user="example", github_unverified=True)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIsNone(receipt)
        self.assertIn("INCOMPLETE", proc.stdout)
        self.assertIn("add identity.github to the manifest", proc.stdout)


STUB = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
root = pathlib.Path(os.environ["FIXTURE_STATE"])
mode = os.environ.get("FIXTURE_MODE", "success")
with (root / "calls.jsonl").open("a") as stream:
    stream.write(json.dumps([name] + args) + "\n")
if name == "git":
    print(os.environ["FIXTURE_ROOT"])
elif name == "sleep":
    pass
elif name == "op":
    if args[:1] == ["whoami"]:
        sys.exit(1 if mode == "skip" else 0)
    if args[:2] == ["group", "list"]:
        marker = root / "op-groups-listed"
        late = mode == "late-group" and marker.exists()
        marker.touch()
        rows = [{"id":"platform","name":"Platform"},{"id":"software","name":"Software"}]
        if late: rows.append({"id":"late","name":"Late"})
        print(json.dumps(rows))
    elif args[:3] == ["group", "user", "list"]:
        group = args[3]
        if group == "late": print('[{"email":"person@example.test"}]')
        else: print("[]" if (root / f"op-revoked-{group}").exists() else '[{"email":"person@example.test"}]')
    elif args[:3] == ["group", "user", "revoke"]:
        group = args[args.index("--group") + 1]
        (root / f"op-revoked-{group}").touch()
elif name == "gh":
    if args[:2] == ["auth", "status"]:
        sys.exit(0)
    if args[:1] == ["api"] and len(args) > 1 and args[1].startswith("users/"):
        if mode == "nonexistent-github": sys.exit(1)
        print('{}')
        sys.exit(0)
    endpoint = next((a for a in args if a.startswith(("orgs/", "repos/"))), "")
    org = endpoint.split("/")[1] if endpoint else ""
    if "members?" in endpoint:
        if not (root / f"gh-member-{org}").exists(): print("example")
    elif "outside_collaborators?" in endpoint:
        if not (root / f"gh-outside-{org}").exists(): print("example")
    elif "/memberships/" in endpoint:
        (root / f"gh-member-{org}").touch()
    elif "/outside_collaborators/" in endpoint:
        (root / f"gh-outside-{org}").touch()
    elif endpoint.startswith("orgs/") and "/invitations/" in endpoint:
        invite = endpoint.rsplit("/", 1)[1]
        if mode != "lingering-invitation": (root / f"gh-org-invite-{org}-{invite}").touch()
    elif endpoint.startswith("repos/") and "/invitations/" in endpoint:
        _, org, repo, _, invite = endpoint.split("/")
        (root / f"gh-repo-invite-{org}-{repo}-{invite}").touch()
    elif endpoint.startswith("orgs/") and "/invitations?" in endpoint:
        if mode == "unreadable-invitations" and org == "first-org": sys.exit(1)
        rows = []
        if org == "first-org" and mode in ("org-invitation", "lingering-invitation") and not (root / f"gh-org-invite-{org}-101").exists():
            rows = [{"id":101,"login":"ExAmPlE","email":None}]
        if org == "first-org" and mode == "email-invitation" and not (root / f"gh-org-invite-{org}-102").exists():
            rows = [{"id":102,"login":None,"email":"PERSON@example.test"}]
        for row in rows: print(json.dumps(row))
    elif endpoint.startswith("orgs/") and "/repos?" in endpoint:
        print(json.dumps({"name":"goat"}))
    elif endpoint.startswith("repos/") and "/invitations?" in endpoint:
        _, org, repo, _ = endpoint.split("/", 3)
        if mode == "repo-invitation" and org == "first-org" and not (root / f"gh-repo-invite-{org}-{repo}-201").exists():
            print(json.dumps({"id":201,"invitee":{"login":"EXAMPLE"}}))
elif name == "goat":
    if "status" in args:
        platform_after = (root / "platform-mutated").exists()
        onboard_after = (root / "onboard-mutated").exists()
        after = platform_after and onboard_after
        if after and mode == "unreadable":
            print("not-json")
            sys.exit(1)
        peer_ids = ["peer-1"] if not onboard_after or mode == "lingering" else []
        print(json.dumps({
          "ory": "inactive []" if onboard_after else "active [PLATFORM-ADMIN]",
          "ory_status": {"read_ok": True, "identity_present": True,
                         "state": "inactive" if onboard_after else "active",
                         "session_count": 0 if onboard_after else 1, "errors": []},
          "mesh_status": {"read_ok": True, "peer_ids": peer_ids, "errors": []},
          "ssh_status": {"read_ok": True,
                         "present": False if mode in ("host-unavailable", "no-keys", "no-keys-boolean-count", "no-keys-float-count", "no-keys-negative-count") else not platform_after,
                         "identifiers": ([] if mode in ("host-unavailable", "no-keys", "no-keys-boolean-count", "no-keys-float-count", "no-keys-negative-count") or platform_after
                                           else ["ssh-1"]), "errors": []},
          "setup_key_status": {"read_ok": True, "present": not onboard_after,
                               "identifiers": [] if onboard_after else ["key-1"], "errors": []},
          "fabric_status": {"read_ok": True,
                            "present": (not onboard_after or mode == "fabric-kept"),
                            "identifiers": (["example-user"]
                              if not onboard_after or mode == "fabric-kept" else []), "errors": []},
          "sponsorship_status": {"read_ok": True, "present": not platform_after,
                                 "identifiers": [] if platform_after else ["example-user"], "errors": []},
          "operator-key": "none" if platform_after else "op_1 active",
          "netbird-role": "user (blocked)" if platform_after else "admin",
          "sponsorship": "no handle on record" if platform_after else "handles example-user",
          "ssh": "1 console row(s) [revoked]; 1 key(s) to act on" if platform_after else "1 console row(s) [installed]; 1 key(s) to act on",
          "read_errors": []}))
    elif "platform-admin" in args and "revoke" in args:
        (root / "platform-mutated").touch()
        no_keys = mode in ("no-keys", "github-fetch-failed", "no-keys-console-row",
                           "no-keys-boolean-count", "no-keys-float-count",
                           "no-keys-negative-count", "no-keys-pre-status-present")
        login = "other-login" if mode == "wrong-github" else "example"
        source = "proposal" if mode == "proposal-github" else "flag"
        keys_checked = (False if mode == "no-keys-boolean-count" else
                        0.0 if mode == "no-keys-float-count" else
                        -1 if mode == "no-keys-negative-count" else
                        0 if no_keys else 1)
        print(json.dumps({"direction": "revoke --full",
                          "results": [{"plane": "ssh", "outcome": "done",
                                       "detail": "host check",
                                       "host_readback": mode != "host-unavailable" and not no_keys,
                                       "github_login": login,
                                       "github_login_source": source,
                                       "github_keys_read_ok": mode != "github-fetch-failed",
                                       "keys_checked": keys_checked,
                                       "console_rows_any_status": 1 if mode == "no-keys-console-row" else (0 if no_keys else 1),
                                       "pubkey_files": 0}]}))
    elif "onboard" in args and "--revoke" in args:
        (root / "onboard-mutated").touch()
'''


if __name__ == "__main__":
    unittest.main()
