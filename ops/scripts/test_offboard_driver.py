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
        for name in ("offboard.sh", "offboard_driver.py", "offboard_subject.py",
                     "offboard_departure.py", "offboard_accounts.py"):
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
               goatnets=None, openbao_user=None, bao_addr=None, door_env=None,
               reason="left-service", reason_text=None, confirmed_by="test-confirmer",
               yes=True, effective=None, stdin=None, account_handles=(), extra_env=None):
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
            if yes:
                argv += ["--yes"]
        for flag, value in (("--reason", reason), ("--reason-text", reason_text),
                            ("--confirmed-by", confirmed_by), ("--effective", effective)):
            if value is not None and (apply or flag != "--confirmed-by"):
                argv += [flag, value]
        for value in account_handles:
            argv += ["--account-handle", value]
        env.update(extra_env or {})
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
            **({"stdin": subprocess.DEVNULL} if stdin is None else {"input": stdin}),
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
            "offboard.preflight", "offboard.host-ssh-keys", "offboard.operator-keys",
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


    # --- the departure record, the reason class and the confirmation (ADR 1272)

    def departure(self):
        paths = sorted((self.root / "ops/departures").glob("*.yml"))
        self.assertEqual(len(paths), 1, paths)
        return paths[0], yaml.safe_load(paths[0].read_text())

    def refused_before_any_call(self, proc, receipt, text):
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn(text, proc.stderr)
        self.assertIsNone(receipt)
        self.assertFalse((self.state / "calls.jsonl").exists()
                         and self.mutations(self.calls()))
        self.assertFalse((self.root / "ops/departures").exists())

    def test_apply_without_a_reason_refuses_before_every_call(self):
        proc, receipt = self.invoke(reason=None)
        self.refused_before_any_call(proc, receipt, "a departure needs --reason")

    def test_reason_other_needs_its_text(self):
        proc, receipt = self.invoke(reason="other")
        self.refused_before_any_call(proc, receipt, "--reason other needs --reason-text")

    def test_apply_without_a_confirmer_refuses_before_every_call(self):
        proc, receipt = self.invoke(confirmed_by=None)
        self.refused_before_any_call(proc, receipt, "a departure needs --confirmed-by")

    def test_confirmer_outside_the_declared_list_is_refused(self):
        declared = self.root / "ops/ansible/group_vars/all.yml"
        declared.write_text(declared.read_text()
                            + 'goat_offboard_confirmers: "first-lead, second-lead"\n')
        proc, receipt = self.invoke()
        self.refused_before_any_call(proc, receipt, "is not one of the declared")
        proc, receipt = self.invoke(confirmed_by="second-lead")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)

    def test_malformed_effective_date_is_refused(self):
        proc, receipt = self.invoke(effective="27 Nov")
        self.refused_before_any_call(proc, receipt, "--effective is a date")

    def test_no_terminal_and_no_yes_removes_nothing(self):
        proc, receipt = self.invoke(yes=False)
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(self.mutations(self.calls()), [])
        self.assertEqual(sum(handler.deletes + handler.puts for handler in self.handlers), 0)
        self.assertIn("no terminal to confirm on", " ".join(receipt["errors"]))
        self.assertEqual(receipt["outcome"], "failed")
        self.assertFalse((self.root / "ops/departures").exists())
        # The question names the person and the counts before anything is removed.
        self.assertIn("person:    example-user", proc.stdout)
        self.assertRegex(proc.stdout, r"found in \d+ of 12 places")

    def test_success_writes_the_record_without_the_login_email(self):
        proc, receipt = self.invoke(effective="2031-02-03")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        path, record = self.departure()
        self.assertEqual(path.name, "example-user-2031-02-03.yml")
        self.assertEqual(receipt["departure_record"], str(path))
        self.assertEqual(record["schema"], "goat-departure/v1")
        self.assertEqual(record["handle"], "example-user")
        self.assertEqual(record["reason"], {"class": "left-service", "text": ""})
        self.assertEqual(record["confirmed_by"], "test-confirmer")
        self.assertEqual(record["recorded_by"], "test-operator")
        self.assertEqual(record["goatnets"], ["test"])
        self.assertNotIn("example.test", path.read_text())
        run = record["runs"][-1]
        self.assertEqual(run["outcome"], "success")
        self.assertEqual(len(run["rows"]), 12)
        self.assertEqual({row["where"] for row in run["rows"]}, {"test", "team"})
        self.assertTrue(all(row["verified"] for row in run["rows"]))
        self.assertLessEqual({row["result"] for row in run["rows"]}, {"removed", "none held"})
        # Leaving the service keeps the purge decision open in the record.
        self.assertEqual(record["state"], "open")
        self.assertEqual([row["id"] for row in record["by_hand"]], ["purge-decision"])
        # One table for the run.
        self.assertEqual(proc.stdout.count("==> result"), 1)
        self.assertRegex(proc.stdout, r"team\s+github\s+\w+\s+(removed|none held)\s+yes")

    def test_closing_the_by_hand_row_completes_the_record(self):
        self.invoke()
        tool = [os.sys.executable, str(self.root / "ops/scripts/offboard_departure.py"),
                "--dir", str(self.root / "ops/departures")]
        missing = subprocess.run(tool + ["close", "example-user", "no-such-row", "--by", "a-lead"],
                                 text=True, capture_output=True)
        self.assertNotEqual(missing.returncode, 0)
        self.assertIn("has no by-hand row", missing.stderr)
        done = subprocess.run(tool + ["close", "example-user", "purge-decision", "--by", "a-lead",
                                      "--note", "login kept"], text=True, capture_output=True)
        self.assertEqual(done.returncode, 0, done.stderr)
        _, record = self.departure()
        self.assertEqual(record["state"], "complete")
        self.assertEqual(record["by_hand"][0]["closed_by"], "a-lead")
        listed = subprocess.run(tool + ["list"], text=True, capture_output=True)
        self.assertIn("example-user", listed.stdout)
        self.assertIn("complete", listed.stdout)

    def test_second_run_adds_a_run_to_the_same_record(self):
        self.invoke("lingering")
        _, record = self.departure()
        self.assertEqual(record["runs"][-1]["outcome"], "failed")
        failed = [row for row in record["runs"][-1]["rows"] if not row["verified"]]
        self.assertTrue(failed)
        self.assertTrue(all(row["result"] in ("FAILED", "NOT RUN") and row["why"]
                            for row in failed))
        self.assertEqual(record["state"], "open")
        proc, _ = self.invoke("lingering")
        _, record = self.departure()
        self.assertEqual(len(record["runs"]), 2)

    def test_a_different_reason_for_the_open_record_is_refused_before_removal(self):
        self.invoke()
        before = len(self.mutations(self.calls()))
        proc, receipt = self.invoke(reason="new-position")
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("gives the reason left-service", " ".join(receipt["errors"]))
        self.assertEqual(len(self.mutations(self.calls())), before)

    def test_new_position_keeps_the_handover_open_and_states_the_return_rule(self):
        proc, _ = self.invoke(reason="new-position")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        _, record = self.departure()
        self.assertEqual([row["id"] for row in record["by_hand"]], ["handover"])
        self.assertIn("a new sponsorship", record["return"])
        self.assertIn("by hand, open: handover", proc.stdout)

    def test_other_reason_records_its_text(self):
        proc, _ = self.invoke(reason="other", reason_text="contract ended")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        _, record = self.departure()
        self.assertEqual(record["reason"], {"class": "other", "text": "contract ended"})
        self.assertEqual(record["by_hand"], [])
        self.assertEqual(record["state"], "complete")

    def test_plan_names_the_record_and_the_reason_rule_and_writes_nothing(self):
        proc, receipt = self.invoke(apply=False, explicit_receipt=False, reason="new-position")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn("departure record: written under", proc.stdout)
        self.assertIn("by hand, kept open in the record: handover", proc.stdout)
        self.assertFalse((self.root / "ops/departures").exists())


    def typed(self, answer):
        """Run --apply without --yes on a pseudo-terminal and type an answer."""
        import pty
        master, slave = pty.openpty()
        env = dict(os.environ)
        env.update(PATH=str(self.bindir) + os.pathsep + os.environ["PATH"],
                   HOME=str(self.home), FIXTURE_ROOT=str(self.root),
                   FIXTURE_STATE=str(self.state), FIXTURE_MODE="success",
                   COLLAB_KANIDM_URL=f"http://127.0.0.1:{self.servers[0].server_port}",
                   COLLAB_KANIDM_IDM_ADMIN_PW="test-password",
                   PUBLIC_DOOR_KANIDM_URL=f"http://127.0.0.1:{self.servers[1].server_port}",
                   GOAT_OFFBOARD_GITHUB_ORGS="", DOOR_PW_UNDER_ANOTHER_NAME="test-password")
        for name in ("GOAT_IDM_ADMIN_PW", "GOAT_PROFILE_DIR", "BAO_ADDR"):
            env.pop(name, None)
        os.write(master, (answer + "\n").encode())
        proc = subprocess.run(
            ["bash", str(self.root / "ops/scripts/offboard.sh"), "example-user",
             "--goatnet", "test", "--apply", "--operator", "test-operator",
             "--reason", "left-service", "--confirmed-by", "test-confirmer",
             "--receipt", str(self.root / "receipt.json")],
            cwd=self.root, env=env, stdin=slave, text=True, capture_output=True, timeout=30)
        os.close(master)
        os.close(slave)
        return proc

    def test_typed_handle_confirms_and_a_wrong_one_removes_nothing(self):
        proc = self.typed("someone-else")
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("the handle typed did not match", proc.stderr)
        self.assertEqual(self.mutations(self.calls()), [])
        self.assertFalse((self.root / "ops/departures").exists())
        proc = self.typed("example-user")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertTrue(self.mutations(self.calls()))
        self.assertEqual(self.departure()[1]["runs"][-1]["outcome"], "success")


    def test_manifest_without_an_org_block_uses_the_handle_as_fabric_identifier(self):
        path = self.root / "ops/operators/test/example-user.yml"
        value = yaml.safe_load(path.read_text())
        value.pop("org")
        path.write_text(yaml.safe_dump(value))
        proc, receipt = self.invoke()
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        status = [call for call in self.calls() if call[:1] == ["goat"] and "status" in call]
        self.assertTrue(status)
        self.assertTrue(all(call[call.index("--org-prefix") + 1] == "example-user"
                            for call in status))

    def test_org_block_without_a_valid_tag_is_still_refused(self):
        path = self.root / "ops/operators/test/example-user.yml"
        value = yaml.safe_load(path.read_text())
        value["org"] = {"display_name": "No Tag"}
        path.write_text(yaml.safe_dump(value))
        proc, receipt = self.invoke()
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(self.mutations(self.calls()), [])
        self.assertIn("manifest has no valid org.short_tag", " ".join(receipt["errors"]))
    # --- external accounts declared by the deployment (ADR 1272 section 3)

    def declare_accounts(self, accounts, plugin=True):
        path = self.root / "ops/offboard/accounts.yaml"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(yaml.safe_dump({"schema": "goat-offboard-accounts/v1",
                                        "accounts": accounts}))
        if plugin:
            script = self.root / "ops/offboard/plugins/synthetic"
            script.parent.mkdir(parents=True, exist_ok=True)
            script.write_text(PLUGIN)
            script.chmod(0o755)

    def plugin_calls(self):
        path = self.state / "plugin.jsonl"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    SYNTHETIC = {"id": "paging", "type": "synthetic", "match": "email",
                 "credential_env": ["SYNTHETIC_TOKEN"]}
    TOKEN = {"SYNTHETIC_TOKEN": "synthetic-value"}

    def test_declared_account_is_checked_removed_and_read_back(self):
        self.declare_accounts([self.SYNTHETIC])
        proc, receipt = self.invoke(extra_env=self.TOKEN)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        row = receipt["accounts"]["paging"]
        self.assertEqual((row["status"], row["before"], row["after"], row["verified"]),
                         ("removed-and-checked", "present", "absent", True))
        calls = self.plugin_calls()
        self.assertEqual([call["verb"] for call in calls], ["check", "remove", "check"])
        self.assertTrue(all(call["identities"] == ["person@example.test"] for call in calls))
        self.assertRegex(proc.stdout, r"account\s+paging\s+present\s+removed\s+yes")
        self.assertRegex(proc.stdout, r"found in \d+ of 13 places")
        record = self.departure()[1]
        self.assertEqual(record["accounts"], ["paging"])
        self.assertIn({"where": "account", "what": "paging", "found": "present",
                       "result": "removed", "verified": True}, record["runs"][-1]["rows"])
        audits = [call for call in self.calls()
                  if call[:1] == ["goat"] and "offboard.account.paging" in call]
        self.assertEqual(len(audits), 1)

    def test_plugin_runs_only_after_every_goatnet_plane(self):
        self.declare_accounts([self.SYNTHETIC])
        self.invoke(extra_env=self.TOKEN)
        self.assertTrue((self.state / "plugin-removed-after-planes").exists())

    def test_account_with_nothing_held_is_not_removed(self):
        self.declare_accounts([self.SYNTHETIC])
        proc, receipt = self.invoke(extra_env={**self.TOKEN, "FIXTURE_ACCOUNT": "absent"})
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertEqual([call["verb"] for call in self.plugin_calls()], ["check", "check"])
        self.assertEqual(receipt["accounts"]["paging"]["before"], "absent")

    def test_declared_plugin_without_its_credential_fails_before_every_removal(self):
        self.declare_accounts([self.SYNTHETIC])
        proc, receipt = self.invoke()
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(self.mutations(self.calls()), [])
        self.assertEqual(self.plugin_calls(), [])
        self.assertEqual(receipt["accounts"]["paging"]["status"], "not-run")
        self.assertIn("missing credential variable(s): SYNTHETIC_TOKEN",
                      receipt["accounts"]["paging"]["detail"])
        self.assertRegex(proc.stdout, r"account\s+paging\s+unknown\s+NOT RUN\s+NO")

    def test_declared_account_without_a_plugin_fails_before_every_removal(self):
        self.declare_accounts([dict(self.SYNTHETIC, type="no-such-plugin")])
        proc, receipt = self.invoke(extra_env=self.TOKEN)
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(self.mutations(self.calls()), [])
        self.assertIn("no executable plug-in", receipt["accounts"]["paging"]["detail"])

    def test_plugin_that_cannot_check_fails_before_every_removal(self):
        self.declare_accounts([self.SYNTHETIC])
        for mode, text in (("crash", "exited 3"), ("garbage", "unreadable JSON"),
                           ("contract", "broke the answer contract")):
            proc, receipt = self.invoke(extra_env={**self.TOKEN, "FIXTURE_ACCOUNT": mode})
            self.assertNotEqual(proc.returncode, 0, mode)
            self.assertEqual(self.mutations(self.calls()), [], mode)
            self.assertIn(text, receipt["accounts"]["paging"]["detail"], mode)

    def test_access_that_survives_the_plugin_fails_the_run(self):
        self.declare_accounts([self.SYNTHETIC])
        proc, receipt = self.invoke(extra_env={**self.TOKEN, "FIXTURE_ACCOUNT": "lingering"})
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(receipt["outcome"], "failed")
        row = receipt["accounts"]["paging"]
        self.assertEqual(row["status"], "check-failed")
        self.assertIn("still held after removal: user 7", row["detail"])
        record = self.departure()[1]
        self.assertEqual(record["state"], "open")
        self.assertNotIn("example.test", self.departure()[0].read_text())

    def test_manual_account_is_listed_and_keeps_the_record_open(self):
        self.declare_accounts([{"id": "site-logins", "type": "manual",
                                "what": "remove the login from each site device"}],
                              plugin=False)
        proc, receipt = self.invoke(reason="other", reason_text="contract ended")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertEqual(receipt["outcome"], "success")
        self.assertRegex(proc.stdout,
                         r"account\s+site-logins\s+not read\s+do by hand\s+-\s+not automated: do by hand")
        self.assertIn("by hand:   site-logins: not automated: do by hand", proc.stdout)
        record = self.departure()[1]
        self.assertEqual(record["state"], "open")
        self.assertEqual(record["by_hand"], [{"id": "site-logins", "state": "open",
                                              "what": "remove the login from each site device"}])
        self.assertEqual(record["runs"][-1]["rows"][-1]["result"], "do by hand")

    def test_declared_handle_match_needs_a_handle_or_an_explicit_none(self):
        account = dict(self.SYNTHETIC, id="cloud", match="declared")
        self.declare_accounts([account])
        proc, receipt = self.invoke(extra_env=self.TOKEN)
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(self.mutations(self.calls()), [])
        self.assertIn("matches by declared handle and none is declared",
                      receipt["accounts"]["cloud"]["detail"])
        proc, receipt = self.invoke(extra_env=self.TOKEN, account_handles=["cloud=-"])
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertEqual(self.plugin_calls(), [])
        self.assertIn("holds none", receipt["accounts"]["cloud"]["detail"])

    def test_manifest_declared_handles_reach_the_plugin(self):
        account = dict(self.SYNTHETIC, id="cloud", match="declared")
        self.declare_accounts([account])
        path = self.root / "ops/operators/test/example-user.yml"
        value = yaml.safe_load(path.read_text())
        value["identity"]["accounts"] = {"cloud": ["first-login", "second-login"]}
        path.write_text(yaml.safe_dump(value))
        proc, _ = self.invoke(extra_env=self.TOKEN, account_handles=["cloud=third-login"])
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertEqual(self.plugin_calls()[0]["identities"],
                         ["first-login", "second-login", "third-login"])

    def test_malformed_declaration_refuses_before_every_call(self):
        for accounts, text in (
                ([{"id": "paging", "type": "manual"}], "needs `what`"),
                ([self.SYNTHETIC, self.SYNTHETIC], "declared twice"),
                ([dict(self.SYNTHETIC, credential_env=["a value, not a name"])],
                 "variable names only"),
                ([dict(self.SYNTHETIC, match="name")], "match must be one of")):
            self.declare_accounts(accounts)
            proc, receipt = self.invoke(extra_env=self.TOKEN)
            self.refused_before_any_call(proc, receipt, text)

    def test_account_handle_for_an_undeclared_account_is_refused(self):
        proc, receipt = self.invoke(account_handles=["nowhere=someone"])
        self.refused_before_any_call(proc, receipt, "names no declared account")

    def test_plan_lists_each_account_and_calls_no_plugin(self):
        self.declare_accounts([self.SYNTHETIC, {"id": "site-logins", "type": "manual",
                                                "what": "remove the login"}])
        proc, _ = self.invoke(apply=False, explicit_receipt=False)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn("paging: plug-in synthetic, matched by email (CANNOT RUN: missing "
                      "credential variable(s): SYNTHETIC_TOKEN)", proc.stdout)
        self.assertIn("site-logins: not automated: do by hand: remove the login", proc.stdout)
        self.assertEqual(self.plugin_calls(), [])

    def test_no_declaration_means_no_account_rows(self):
        proc, receipt = self.invoke()
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertEqual(receipt["accounts"], {})
        self.assertEqual(receipt["by_hand"], [])
    def test_empty_group_printed_as_null_is_a_successful_read(self):
        # The 1Password CLI prints `null` for a group with no members.
        proc, receipt = self.invoke("op-null-empty-group")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        row = receipt["goatnets"]["test"]["onepassword"]
        self.assertEqual((row["status"], row["before"], row["after"]),
                         ("removed-and-checked", "present", "absent"))

    def test_group_read_that_is_not_a_list_still_fails_before_removal(self):
        proc, receipt = self.invoke("op-unreadable-group")
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(self.mutations(self.calls()), [])
        self.assertIn("cannot read 1Password group", " ".join(receipt["errors"]))


    def test_unwritable_audit_stops_the_run_before_any_removal(self):
        proc, receipt = self.invoke("audit-unwritable")
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(self.mutations(self.calls()), [])
        self.assertEqual(sum(handler.deletes + handler.puts for handler in self.handlers), 0)
        self.assertIn("an operator-action entry cannot be written, so nothing was removed: "
                      "router refused the certificate", " ".join(receipt["errors"]))
        self.assertFalse((self.root / "ops/departures").exists())

    def test_audit_is_tried_only_after_every_other_preflight_check_passes(self):
        proc, _ = self.invoke("skip")
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual([call for call in self.calls() if call[:1] == ["goat"] and "audit" in call], [])


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
        elif mode == "op-unreadable-group": print('{"error": "not a list"}')
        else: print(("null" if mode == "op-null-empty-group" else "[]") if (root / f"op-revoked-{group}").exists() else '[{"email":"person@example.test"}]')
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
    if "audit" in args and mode == "audit-unwritable":
        print("router refused the certificate: unknown issuer", file=sys.stderr)
        sys.exit(1)
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


PLUGIN = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
verb = sys.argv[1]
state = pathlib.Path(os.environ["FIXTURE_STATE"])
mode = os.environ.get("FIXTURE_ACCOUNT", "held")
request = json.load(sys.stdin)
assert os.environ["SYNTHETIC_TOKEN"] == "synthetic-value"
with (state / "plugin.jsonl").open("a") as stream:
    stream.write(json.dumps({"verb": verb, "identities": request["identities"],
                             "account": request["account"]["id"]}) + "\n")
if mode == "crash":
    print("synthetic service refused", file=sys.stderr); sys.exit(3)
if mode == "garbage":
    print("not json"); sys.exit(0)
if mode == "contract":
    print(json.dumps({"held": False, "items": ["user 7"]})); sys.exit(0)
gone = state / "plugin-removed"
if verb == "remove":
    if mode != "lingering": gone.touch()
    if (state / "onboard-mutated").exists() or (state / "platform-mutated").exists():
        (state / "plugin-removed-after-planes").touch()
held = mode != "absent" and not gone.exists()
print(json.dumps({"held": held, "items": ["user 7"] if held else [],
                  "detail": "one user" if held else "no user"}))
'''


if __name__ == "__main__":
    unittest.main()
