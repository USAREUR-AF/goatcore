#!/usr/bin/env python3
"""No live accounts, credentials, operator files or hosts used."""
import copy
import base64
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest
import uuid
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
loader = importlib.util.spec_from_file_location("secret_store", ROOT / "ops/scripts/newsite-secret-store.py")
store = importlib.util.module_from_spec(loader)
loader.loader.exec_module(store)


class Generator(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        scripts = self.root / "ops/scripts"
        scripts.mkdir(parents=True)
        for name in ("newsite-secrets.sh", "newsite-secret-store.py"):
            shutil.copy(ROOT / "ops/scripts" / name, scripts)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        self.directory = self.root / ".envrc.local.d"
        self.directory.mkdir(mode=0o700)
        self.target = self.directory / "30-site-services.refs"
        self.adapter = self.root / "fake-store"
        self.adapter.write_text('''#!/usr/bin/env python3
import json,sys
payload=json.load(sys.stdin)
json.dump({name: 'test://' + payload['site'] + '/' + name for name in payload['secrets']}, sys.stdout)
''')
        self.adapter.chmod(0o700)
        # Tests must not inherit a live operator's credential variables.
        self.env = {"PATH": os.environ["PATH"], "HOME": str(self.root),
                    "GOAT_SECRET_STORE": "custom", "GOAT_SECRET_INIT": str(self.adapter),
                    "GOAT_SECRET_REF_PREFIX": "test://"}

    def run_script(self, *args):
        return subprocess.run([os.environ.get("BASH_UNDER_TEST", "/bin/bash"),
                               str(self.root / "ops/scripts/newsite-secrets.sh"), "fixture-site", *args],
                              env=self.env, capture_output=True, text=True)

    def test_missing_client_and_cookie_families(self):
        result = self.run_script("--spec-json")
        self.assertEqual(result.returncode, 0, result.stderr)
        spec = json.loads(result.stdout)
        for family in ("STETHOSCOPE_OIDC", "COMPLIANCE_OIDC", "MICROSCOPE_OAUTH2_PROXY",
                       "CONSISTENCY_CONSOLE_OAUTH2_PROXY", "GOAT_MAP_OAUTH2_PROXY", "CONSISTENCY_AGGREGATOR_OAUTH2_PROXY"):
            self.assertEqual(spec[f"FIXTURE_SITE_{family}_CLIENT_SECRET"], "hex48")
        for family in ("COMPLIANCE", "MICROSCOPE_OAUTH2_PROXY", "CONSISTENCY_CONSOLE_OAUTH2_PROXY",
                       "CONSISTENCY_AGGREGATOR_OAUTH2_PROXY"):
            self.assertEqual(spec[f"FIXTURE_SITE_{family}_COOKIE_SECRET"], "hex32")

    def test_reference_only_and_idempotent(self):
        microscope = "FIXTURE_SITE_MICROSCOPE_AUDIT_S3_SECRET_KEY"
        spec = json.loads(self.run_script("--spec-json").stdout)
        self.assertEqual(spec[microscope], "hex40")
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        before = self.target.read_bytes()
        self.assertEqual(self.target.stat().st_mode & 0o777, 0o600)
        names = store.assignments(self.target, "test://")
        self.assertEqual(len(names), 46)
        self.assertIn(microscope, names)
        self.assertIn(f'{microscope}="test://fixture-site/{microscope}"', self.target.read_text())
        self.adapter.unlink()  # A complete re-run must not contact the store.
        self.assertEqual(self.run_script().returncode, 0)
        self.assertEqual(self.target.read_bytes(), before)
        self.assertFalse(list(self.directory.glob("*.envrc")))

    def test_existing_legacy_and_other_refs_not_rotated(self):
        legacy = self.directory / "legacy.envrc"
        legacy.write_text('export FIXTURE_SITE_KRATOS_CIPHER="existing-test-value"\n')
        refs = self.directory / "elsewhere.refs"
        refs.write_text('FIXTURE_SITE_HYDRA_SYSTEM="test://existing/hydra"\n')
        self.env["FIXTURE_SITE_NB_ENCKEY"] = "existing-test-environment-value"
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        names = store.assignments(self.target, "test://")
        self.assertEqual(len(names), 43)
        self.assertFalse({"FIXTURE_SITE_KRATOS_CIPHER", "FIXTURE_SITE_HYDRA_SYSTEM", "FIXTURE_SITE_NB_ENCKEY"} & names)
        self.assertNotIn("existing-test-value", result.stdout + result.stderr)
        self.assertIn("existing-test-value", legacy.read_text())

    def test_dry_run_no_store_no_files(self):
        self.adapter.unlink()
        result = self.run_script("--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("46 to generate", result.stdout)
        self.assertEqual(list(self.directory.iterdir()), [])

    def test_plaintext_destination_refused(self):
        result = self.run_script("--file", str(self.directory / "bad.envrc"))
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(list(self.directory.iterdir()), [])

    def test_failed_provider_does_not_echo_or_write(self):
        self.adapter.write_text('#!/bin/sh\necho sensitive-test-marker >&2\nexit 1\n')
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("sensitive-test-marker", result.stdout + result.stderr)
        self.assertEqual(list(self.directory.iterdir()), [])

    def test_invalid_provider_reference_refused(self):
        self.adapter.write_text('''#!/usr/bin/env python3
import json,sys
json.dump({key:'plaintext-test-marker' for key in json.load(sys.stdin)['secrets']},sys.stdout)
''')
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("plaintext-test-marker", result.stdout + result.stderr)
        self.assertEqual(list(self.directory.iterdir()), [])

    def test_symlink_and_existing_plaintext_refs_refused(self):
        outside = self.root / "outside"
        outside.write_text("untouched")
        self.target.symlink_to(outside)
        self.assertNotEqual(self.run_script().returncode, 0)
        self.assertEqual(outside.read_text(), "untouched")
        self.target.unlink()
        self.target.write_text('export FIXTURE_SITE_KRATOS_CIPHER="plaintext-test-marker"\n')
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("plaintext-test-marker", result.stdout + result.stderr)

    def test_duplicate_and_locked_refs_refused(self):
        self.target.write_text('A="test://a"\nA="test://b"\n')
        self.assertNotEqual(self.run_script().returncode, 0)
        self.target.unlink()
        lock = self.target.with_name(self.target.name + ".lock")
        lock.mkdir()
        self.assertNotEqual(self.run_script().returncode, 0)
        self.assertTrue(lock.exists())

    def test_bootstrap_pair_requires_explicit_handle(self):
        spec = json.loads(self.run_script("--spec-json").stdout)
        self.assertEqual(spec["FIXTURE_SITE_PORTAL_OPERATOR_API_KEY"], "b64_32")
        self.assertEqual(spec["FIXTURE_SITE_PORTAL_OPERATOR_API_KEYS"], "pair:")
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("FIXTURE_SITE_PORTAL_OPERATOR_API_KEYS", store.assignments(self.target, "test://"))

    def test_explicit_pair_and_existing_roster_not_replaced(self):
        result = self.run_script("--bootstrap-operator", "alice")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("FIXTURE_SITE_PORTAL_OPERATOR_API_KEYS", store.assignments(self.target, "test://"))
        before = self.target.read_bytes()
        result = self.run_script("--bootstrap-operator", "bob")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.target.read_bytes(), before)
        self.assertIn("audited platform-admin", result.stderr)

    def test_existing_plural_does_not_gain_new_legacy_authority(self):
        self.env["FIXTURE_SITE_PORTAL_OPERATOR_API_KEYS"] = "alice:test-only-existing-key"
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        names = store.assignments(self.target, "test://")
        self.assertNotIn("FIXTURE_SITE_PORTAL_OPERATOR_API_KEY", names)
        self.assertNotIn("FIXTURE_SITE_PORTAL_OPERATOR_API_KEYS", names)


    def test_bad_bootstrap_handles_rejected_before_store(self):
        self.adapter.unlink()
        for handle in ("", "operator", "alice:bob", "alice,bob", " a ", "alice\n", "a" * 65):
            result = self.run_script("--bootstrap-operator", handle)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(self.target.exists())


class OnePassword(unittest.TestCase):
    def setUp(self):
        self.items = []
        self.calls = []

    def provider(self, argv, payload=None):
        self.calls.append((argv, copy.deepcopy(payload)))
        if argv[1:3] == ["vault", "get"]:
            return {"id": "vault-id"}
        operation = argv[2]
        if operation == "list":
            return copy.deepcopy(self.items)
        if operation == "get":
            return copy.deepcopy(self.items[0])
        if operation in ("create", "edit"):
            item = copy.deepcopy(payload)
            item["id"] = "item-id"
            self.items[:] = [item]
            return item
        return self.fail("unexpected provider operation")

    def test_create_retry_and_extend_preserves_values(self):
        with patch.object(store, "run_json", self.provider):
            refs = store.onepassword("fixture", {"SITE_FIRST": "hex48"}, "chosen-vault")
            first = self.items[0]["fields"][0]["value"]
            self.assertEqual(len(first), 48)
            self.assertEqual(refs, {"SITE_FIRST": "op://vault-id/item-id/SITE_FIRST"})
            store.onepassword("fixture", {"SITE_FIRST": "hex48"}, "chosen-vault")
            self.assertEqual(len([argv for argv, _ in self.calls if argv[2] == "create"]), 1)
            store.onepassword("fixture", {"SITE_FIRST": "hex48", "SITE_NEXT": "hex32"}, "chosen-vault")
            self.assertEqual(self.items[0]["fields"][0]["value"], first)
            self.assertEqual(len(self.items[0]["fields"][1]["value"]), 32)
            for argv, payload in self.calls:
                self.assertNotIn(first, " ".join(argv))
                if argv[2] == "create":
                    self.assertEqual(argv[-1], "-")
                    self.assertIsNotNone(payload)

    def test_explicit_vault_required(self):
        with self.assertRaises(store.Failure):
            store.onepassword("fixture", {"SITE_FIRST": "hex48"}, None)

    def test_generator_formats_preserved(self):
        for length in (32, 40, 48):
            value = store.generate(f"hex{length}")
            self.assertRegex(value, "^[0-9a-f]+$")
            self.assertEqual(len(value), length)
        for count in (32, 36):
            self.assertEqual(len(base64.b64decode(store.generate(f"b64_{count}"), validate=True)), count)
        self.assertEqual(uuid.UUID(store.generate("uuid")).version, 4)
        self.assertEqual(store.generate("literal:fixture-minio-root"), "fixture-minio-root")
        pair = store.generate("pair:alice")
        handle, key = pair.split(":")
        self.assertEqual(handle, "alice")
        self.assertEqual(len(base64.b64decode(key, validate=True)), 32)

    def test_stored_pair_retry_preserves_handle_and_key(self):
        with patch.object(store, "run_json", self.provider):
            store.onepassword("fixture", {"SITE_PAIRS": "pair:alice"}, "vault")
            first = self.items[0]["fields"][0]["value"]
            self.assertTrue(first.startswith("alice:"))
            store.onepassword("fixture", {"SITE_PAIRS": "pair:alice"}, "vault")
            self.assertEqual(self.items[0]["fields"][0]["value"], first)
            with self.assertRaises(store.Failure):
                store.onepassword("fixture", {"SITE_PAIRS": "pair:bob"}, "vault")
            self.items[0]["fields"][0]["value"] = "alice:test-only-key:extra"
            with self.assertRaises(store.Failure):
                store.onepassword("fixture", {"SITE_PAIRS": "pair:alice"}, "vault")

    def test_duplicate_items_and_unmanaged_item_refused(self):
        title = "goat site fixture services"
        with patch.object(store, "run_json", self.provider):
            self.items = [{"id": "one", "title": title}, {"id": "two", "title": title}]
            with self.assertRaises(store.Failure):
                store.onepassword("fixture", {"SITE_FIRST": "hex48"}, "vault")
            self.items = [{"id": "one", "title": title, "category": "LOGIN"}]
            with self.assertRaises(store.Failure):
                store.onepassword("fixture", {"SITE_FIRST": "hex48"}, "vault")

    def test_empty_or_duplicate_stored_field_refused(self):
        with patch.object(store, "run_json", self.provider):
            store.onepassword("fixture", {"SITE_FIRST": "hex48"}, "vault")
            self.items[0]["fields"].append(copy.deepcopy(self.items[0]["fields"][0]))
            with self.assertRaises(store.Failure):
                store.onepassword("fixture", {"SITE_FIRST": "hex48"}, "vault")

            self.items[0]["fields"].pop()
            self.items[0]["fields"][0]["value"] = ""
            with self.assertRaises(store.Failure):
                store.onepassword("fixture", {"SITE_FIRST": "hex48"}, "vault")

    def test_failed_reference_replace_retry_reuses_stored_secret(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            refs_dir = root / ".envrc.local.d"
            refs_dir.mkdir(mode=0o700)
            target = refs_dir / "30-site-services.refs"
            target.write_text('SITE_KEEP="op://vault-id/old-item/keep"\n')
            previous = target.read_bytes()
            # Pin the store. This test exercises the reference path, which runs
            # only when the checkout declares 1Password; without the pin the
            # test reads whatever the checkout happens to declare, and a
            # checkout holding a goatlite.yaml declares env-file (ADR 1213 D4,
            # ops/secrets/posture.py _lite_declared). The value path then takes
            # the first os.replace this test patches to fail, so the failure
            # arrives before the store is ever called and reads as a confusing
            # IndexError on an empty item list.
            with patch.dict(os.environ, {"GOAT_SECRET_VAULT": "vault",
                                         "GOAT_SECRET_STORE": "1password"}, clear=True), \
                 patch.object(store.sys, "argv", ["newsite-secret-store.py", "fixture"]), \
                 patch.object(store.subprocess, "check_output", return_value=str(root / ".git")), \
                 patch.object(store, "run_json", self.provider):
                with patch.object(store.sys, "stdin", io.StringIO("SITE_FIRST|hex48|\n")), \
                     patch.object(store.os, "replace", side_effect=OSError("test-only write failure")):
                    with self.assertRaises(OSError):
                        store.main()
                first = self.items[0]["fields"][0]["value"]
                self.assertEqual(target.read_bytes(), previous)
                self.assertEqual(list(refs_dir.iterdir()), [target])
                with patch.object(store.sys, "stdin", io.StringIO("SITE_FIRST|hex48|\n")), \
                     patch.object(store.sys, "stdout", io.StringIO()):
                    store.main()
                self.assertEqual(self.items[0]["fields"][0]["value"], first)
                self.assertNotIn(first, target.read_text())
                self.assertEqual(len(store.assignments(target, "op://")), 2)
                self.assertEqual(target.stat().st_mode & 0o777, 0o600)



class WorktreeCredentials(unittest.TestCase):
    """Exercise generation, worktree fallback and injection without operator files."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="goat-worktree-secrets-")
        self.addCleanup(self.tmp.cleanup)
        self.directory = Path(self.tmp.name)
        self.main = self.directory / "main"
        subprocess.run(["git", "clone", "-q", "--shared", "--no-checkout", str(ROOT), str(self.main)], check=True)
        scripts = self.main / "ops/scripts"
        scripts.mkdir(parents=True)
        for name in ("newsite-secrets.sh", "newsite-secret-store.py", "ansible-preflight.sh", "fresh-source.sh", "with-secrets"):
            shutil.copy(ROOT / "ops/scripts" / name, scripts / name)
        self.worktree = self.directory / "worktree"
        subprocess.run(["git", "-C", str(self.main), "worktree", "add", "-q", "--no-checkout",
                        "--detach", str(self.worktree), "HEAD"], check=True)
        shutil.copytree(self.main / "ops", self.worktree / "ops")
        self.credentials = self.main / ".envrc.local.d"
        self.credentials.mkdir(mode=0o700)
        self.references = self.credentials / "30-site-services.refs"
        self.bin = self.directory / "bin"
        self.bin.mkdir()
        self.calls = self.directory / "injector.calls"
        self.executable("ansible-playbook", r"""#!/usr/bin/env python3
import json, os, sys
value = os.environ.get("FIXTURE_SITE_PG_PASS", "")
if "--list-tasks" in sys.argv:
    print("      fixture task TAGS: [fixture]" if value == "fixture-resolved" else "  play #1 TAGS: [fixture]")
else:
    print(json.dumps({"present": bool(value), "resolved": value == "fixture-resolved",
                      "legacy": value == "fixture-legacy", "inherited": value == "fixture-inherited"}))
""")
        self.executable("op", r"""#!/usr/bin/env python3
import os, pathlib, sys
assert sys.argv[1:3] == ["run", "--"]
path = pathlib.Path(os.environ["FIXTURE_CALLS"])
assert not path.exists(), "injector must run only once"
path.write_text("run\n")
prefix = os.environ.get("GOAT_SECRET_REF_PREFIX", "op://")
for name, value in list(os.environ.items()):
    if value.startswith(prefix) and len(value) > len(prefix):
        os.environ[name] = "fixture-resolved"
os.execvp(sys.argv[3], sys.argv[3:])
""")
        self.adapter = self.executable("init-store", r"""#!/usr/bin/env python3
import json, os, sys
payload = json.load(sys.stdin)
prefix = os.environ.get("GOAT_SECRET_REF_PREFIX", "op://")
json.dump({name: prefix + "fixture/item/" + name for name in payload["secrets"]}, sys.stdout)
""")
        self.env = {"PATH": str(self.bin) + os.pathsep + os.environ["PATH"], "HOME": str(self.directory),
                    "GOAT_SECRET_STORE": "custom", "GOAT_SECRET_INIT": str(self.adapter),
                    "GOAT_SECRET_RUN": str(self.bin / "op") + " run --",
                    "GOAT_REFERENCES_ONLY": "1", "FIXTURE_CALLS": str(self.calls),
                    # The worktree is judged against its own HEAD: this exercises the
                    # credential paths, not freshness against a trunk (contract C3).
                    "GOAT_FRESH_SOURCE_TRUNK": subprocess.run(["git", "-C", str(ROOT), "rev-parse", "HEAD"],
                                                              capture_output=True, text=True, check=True).stdout.strip()}

    def executable(self, name, source):
        path = self.bin / name
        path.write_text(source)
        path.chmod(0o700)
        return path

    def generate(self):
        result = subprocess.run([str(self.worktree / "ops/scripts/newsite-secrets.sh"), "fixture-site"],
                                cwd=self.worktree, env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.references.exists())
        self.assertFalse((self.worktree / ".envrc.local.d").exists())

    def preflight(self, *args):
        return subprocess.run([str(self.worktree / "ops/scripts/ansible-preflight.sh"), "-i", "localhost,", "fixture.yml", *args],
                              cwd=self.worktree, env=self.env, capture_output=True, text=True)

    def consumer(self):
        result = self.preflight()
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_generated_references_reach_worktree_and_inject_once(self):
        self.generate()
        self.assertTrue(self.consumer()["resolved"])
        self.assertEqual(self.calls.read_text(), "run\n")

    def test_staleness_bypass_still_resolves_references(self):
        self.generate()
        self.env["GOAT_SKIP_ANSIBLE_PREFLIGHT"] = "1"
        self.assertTrue(self.consumer()["resolved"])

    def test_task_listing_guard_receives_resolved_values(self):
        self.generate()
        result = self.preflight("--tags", "fixture")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)["resolved"])

    def test_default_onepassword_injector(self):
        self.generate()
        self.env["GOAT_SECRET_STORE"] = "1password"
        self.assertTrue(self.consumer()["resolved"])
        self.assertTrue(self.calls.exists())

    def test_custom_prefix_is_loaded_and_resolved(self):
        self.env["GOAT_SECRET_REF_PREFIX"] = "fixture://"
        self.generate()
        self.assertTrue(self.consumer()["resolved"])

    def test_references_only_never_sources_legacy_file(self):
        self.generate()
        marker = self.directory / "legacy-sourced"
        (self.credentials / "legacy.envrc").write_text(
            f'touch "{marker}"\nexport FIXTURE_SITE_PG_PASS="fixture-legacy"\n')
        self.assertTrue(self.consumer()["resolved"])
        self.assertFalse(marker.exists())

    def test_legacy_migration_still_wins(self):
        # A value a legacy file sets is not a reference, so it reaches the play
        # as it is; the references still in the environment are resolved rather
        # than handed to the play as literal op:// strings.
        self.env["GOAT_REFERENCES_ONLY"] = "0"
        self.generate()
        (self.credentials / "legacy.envrc").write_text('export FIXTURE_SITE_PG_PASS="fixture-legacy"\n')
        self.assertTrue(self.consumer()["legacy"])

    def test_surrounding_injector_values_are_preserved(self):
        self.generate()
        self.env.update({name: "fixture-inherited" for name in store.assignments(self.references, "op://")})
        self.env["GOAT_SECRETS_RESOLVED"] = "1"
        self.assertTrue(self.consumer()["inherited"])
        self.assertFalse(self.calls.exists())

    def test_worktree_local_directory_suppresses_master_fallback(self):
        self.generate()
        (self.worktree / ".envrc.local.d").mkdir()
        self.assertFalse(self.consumer()["present"])
        self.assertFalse(self.calls.exists())

    def test_malformed_file_refuses_without_execution_or_disclosure(self):
        marker = self.directory / "shell-executed"
        valid = 'FIXTURE_SITE_PG_PASS="op://fixture/item/pass"\n'
        invalid = [
            'OTHER="TEST_ONLY_VALUE_MUST_NOT_APPEAR"\n',
            f'OTHER="op://fixture/$(touch {marker})"\n',
            valid,
            'OTHER="op://fixture/item/unclosed\n',
        ]
        for line in invalid:
            with self.subTest(line=line):
                self.references.write_text(valid + line)
                result = self.preflight()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("values withheld", result.stderr)
                self.assertNotIn("TEST_ONLY_VALUE_MUST_NOT_APPEAR", result.stdout + result.stderr)
                self.assertFalse(marker.exists())
                self.assertFalse(self.calls.exists())
                self.assertEqual(result.stdout, "")

    def test_symlinked_reference_file_refused(self):
        outside = self.directory / "outside.refs"
        outside.write_text('FIXTURE_SITE_PG_PASS="op://fixture/item/pass"\n')
        self.references.symlink_to(outside)
        result = self.preflight()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("values withheld", result.stderr)
        self.assertFalse(self.calls.exists())


class GoatlitePresetCoverage(unittest.TestCase):
    """Every secret a goatlite goatnet's plays read must be one phase 3 mints.

    Two independent lists decide this and nothing compared them. The plays read
    names bound in the goatnet's group_vars, which ops/scripts/goatlite-render.py
    writes. Phase 3 mints values from a hardcoded SPEC in newsite-secrets.sh. A
    goatlite learner has no other source: no operator shell history, no vault
    someone filled in years ago.

    SPIRE_DB_PASS was on the first list and not the second. The consequence was
    not a missing-variable error at phase 3 but a Postgres role created with no
    password at all (the task carries no_log), and then a failure five minutes
    later inside docker compose reading "required variable SPIRE_DB_PASSWORD is
    missing a value" -- naming the symptom, one layer below the cause.
    """

    def _bound_by_the_renderer(self):
        text = (ROOT / "ops/scripts/goatlite-render.py").read_text(encoding="utf-8")
        return set(re.findall(r"lookup\('env', '\{up\}_([A-Z0-9_]+)'\)", text))

    def _minted_by_phase_3(self):
        text = (ROOT / "ops/scripts/newsite-secrets.sh").read_text(encoding="utf-8")
        return set(re.findall(r"^\$\{S\}_([A-Z0-9_]+)\|", text, re.M))

    def test_both_lists_are_still_readable(self):
        # If either pattern stops matching, the comparison below passes vacuously
        # and the drift it exists to catch comes back silently.
        self.assertTrue(self._bound_by_the_renderer(),
                        "no per-site env bindings found in goatlite-render.py: "
                        "the lookup('env', '{up}_NAME') shape must have changed, "
                        "and this test no longer checks anything")
        self.assertTrue(self._minted_by_phase_3(),
                        "no SPEC entries found in newsite-secrets.sh: the "
                        "${S}_NAME|kind|note shape must have changed, and this "
                        "test no longer checks anything")

    def test_every_secret_the_plays_read_is_one_phase_3_mints(self):
        missing = sorted(self._bound_by_the_renderer() - self._minted_by_phase_3())
        self.assertEqual([], missing, "a goatlite goatnet's plays read "
                         f"{', '.join('<SITE>_' + n for n in missing)} from the "
                         "environment, and phase 3 never creates it. Add it to "
                         "SPEC in ops/scripts/newsite-secrets.sh, or stop "
                         "binding it in ops/scripts/goatlite-render.py.")


if __name__ == "__main__":
    unittest.main()
