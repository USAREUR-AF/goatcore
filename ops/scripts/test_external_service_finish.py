#!/usr/bin/env python3
"""Tests for the governed-scope branch of external-service-finish.py.

mesh-change-control.md §4.5: on a scope whose ops/warrant/membership/<scope>/
_scope.yaml enforces, the finish writes the host's `exact: [<P>-EXTERNAL-INFRA]`
entry into that scope's services.yaml (or in place, in whichever fragment
already holds the peer) on the PR and never writes NetBird live. On any other scope it writes live, as
before. On every scope the group name comes from the taxonomy's site_prefix (#1451).
"""
import base64
import contextlib
import datetime
import importlib.util
import io
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from contextlib import ExitStack
from pathlib import Path
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
spec = importlib.util.spec_from_file_location("external_service_finish", HERE / "external-service-finish.py")
esf = importlib.util.module_from_spec(spec)
sys.modules["external_service_finish"] = esf
spec.loader.exec_module(esf)

TODAY = datetime.date(2026, 9, 23)


def base_facts(site):
    # What site_facts() hands governance(): the env-var prefix, and no group name.
    return {"site_id": site, "prefix": site.upper().replace("-", "_")}


class GovernanceTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "ops" / "warrant" / "membership").mkdir(parents=True)
        (self.root / "ops" / "ansible").mkdir(parents=True)
        shutil.copy(REPO / "ops" / "ansible" / "netbird_acl_taxonomy.yml", self.root / "ops" / "ansible")
        self.sdir = self.root / "ops" / "warrant" / "membership" / "zagros-dev"
        self.sdir.mkdir()
        self.scope = self.sdir / "_scope.yaml"
        self.scope.write_text("# header\nversion: 1\nscope: zagros-dev\nconverge: enforce\n")
        (self.sdir / "_baseline.generated.yaml").write_text(
            "version: 1\nscope: zagros-dev\ngenerated_from: test\nidentity_hashes: {}\npeople: {}\n")
        self.mfile = self.sdir / "services.yaml"
        self.st = {"peer_name": "vendor-registry-01", "name": "registry", "pr": 1234, "owner": "user369x",
                   "retire_after": "2026-12-31"}

    def tearDown(self):
        self.tmp.cleanup()

    def test_report_file_is_not_governed(self):
        """A `converge: report` file is an adoption draft, not the switch (mesh-change-control §4.1)."""
        self.scope.write_text("version: 1\nscope: zagros-dev\nconverge: report\n")
        self.assertIsNone(esf.warrant_membership.governed_dir("zagros-dev", self.sdir.parent))
        self.assertFalse(esf.governance(str(self.root), base_facts("zagros-dev"))["governed"])

    def test_unreadable_converge_counts_as_governed(self):
        self.scope.write_text("version: 1\nscope: zagros-dev\n")
        self.assertTrue(esf.governance(str(self.root), base_facts("zagros-dev"))["governed"])

    def test_retired_single_file_is_refused(self):
        (self.sdir.parent / "zagros.yaml").write_text("version: 1\nscope: zagros\nconverge: enforce\nentries: []\n")
        with self.assertRaises(esf.Stop):
            esf.governance(str(self.root), base_facts("zagros"))

    def test_governed_scope_uses_taxonomy_prefix(self):
        f = esf.governance(str(self.root), base_facts("zagros-dev"))
        self.assertTrue(f["governed"])
        self.assertEqual(f["external_infra"], "ZAGROS-DEV-EXTERNAL-INFRA")

    def test_ungoverned_scope_uses_taxonomy_prefix(self):
        f = esf.governance(str(self.root), base_facts("zagros"))
        self.assertFalse(f["governed"])
        self.assertEqual(f["external_infra"], "ZAGROS-EXTERNAL-INFRA")

    def test_hyphenated_ungoverned_goatnet_keeps_its_hyphen(self):
        # #1451: the env-var prefix of site-a-b is SITE_A_B; its group is
        # SITE-A-B-EXTERNAL-INFRA, never SITE_A_B-EXTERNAL-INFRA.
        facts = base_facts("zagros-prod")
        self.assertEqual(facts["prefix"], "ZAGROS_PROD")
        f = esf.governance(str(self.root), facts)
        self.assertFalse(f["governed"])
        self.assertEqual(f["site_prefix"], "ZAGROS-PROD")
        self.assertEqual(f["external_infra"], "ZAGROS-PROD-EXTERNAL-INFRA")
        self.assertEqual(f["nb_token_env"] if "nb_token_env" in f else "ZAGROS_PROD_NB_API_TOKEN",
                         "ZAGROS_PROD_NB_API_TOKEN")

    def test_site_facts_hyphenated_goatnet(self):
        # The whole path from group_vars: env names keep the underscore, the
        # group name takes the taxonomy's hyphenated site_prefix.
        gv = self.root / "ops" / "ansible" / "group_vars" / "zagros_prod_hosts"
        gv.mkdir(parents=True)
        (gv / "vars.yml").write_text("site_id: zagros-prod\nsite_dns_zone: example.net\n")
        f = esf.site_facts(str(self.root), "zagros_prod_hosts")
        self.assertEqual(f["external_infra"], "ZAGROS-PROD-EXTERNAL-INFRA")
        self.assertEqual(f["nb_token_env"], "ZAGROS_PROD_NB_API_TOKEN")

    def test_scope_missing_from_taxonomy_is_refused(self):
        with self.assertRaises(esf.Stop):
            esf.governance(str(self.root), base_facts("no-such-goatnet"))

    def test_entry_shape(self):
        f = esf.governance(str(self.root), base_facts("zagros-dev"))
        e = esf.external_infra_entry(self.st, f, TODAY)
        self.assertEqual(e["exact"], ["ZAGROS-DEV-EXTERNAL-INFRA"])
        self.assertEqual(e["expires"], "2026-12-31")
        self.assertLessEqual(len(e["reason"]), 200)

    def test_entry_expiry_defaults_when_none_or_past(self):
        f = esf.governance(str(self.root), base_facts("zagros-dev"))
        for ra in (None, "2020-01-01"):
            st = dict(self.st, retire_after=ra)
            e = esf.external_infra_entry(st, f, TODAY)
            self.assertEqual(e["expires"], (TODAY + datetime.timedelta(days=180)).isoformat())

    def test_write_then_replace_entry(self):
        f = esf.governance(str(self.root), base_facts("zagros-dev"))
        rel, how = esf.write_membership_entry(str(self.root), self.st, f, TODAY)
        self.assertEqual((rel, how), ("ops/warrant/membership/zagros-dev/services.yaml", "added"))
        _, how = esf.write_membership_entry(str(self.root), self.st, f, TODAY)
        self.assertEqual(how, "unchanged")
        _, how = esf.write_membership_entry(str(self.root), dict(self.st, retire_after="2027-01-31"), f, TODAY)
        self.assertEqual(how, "replaced")
        self.assertEqual(esf.warrant_membership.check_scope(self.sdir, today=TODAY), [])
        doc = esf.warrant_membership.load(self.mfile)
        self.assertEqual(len(doc["entries"]), 1)
        self.assertIn("# header", self.scope.read_text())

    def test_existing_entry_is_edited_in_its_own_fragment(self):
        """One entry per peer across fragments: a peer already declared in
        people.yaml is rewritten there, never duplicated into services.yaml."""
        people = self.sdir / "people.yaml"
        people.write_text("version: 1\nentries:\n  - peer: vendor-registry-01\n"
                          "    absent: [ZAGROS-DEV-OBSERVER]\n    owner: o\n    reason: r\n")
        f = esf.governance(str(self.root), base_facts("zagros-dev"))
        rel, how = esf.write_membership_entry(str(self.root), self.st, f, TODAY)
        self.assertEqual((rel, how), ("ops/warrant/membership/zagros-dev/people.yaml", "replaced"))
        self.assertFalse(self.mfile.exists())
        self.assertEqual(esf.warrant_membership.load(people)["entries"][0]["exact"], ["ZAGROS-DEV-EXTERNAL-INFRA"])

    def test_invalid_entry_is_refused_and_file_restored(self):
        f = esf.governance(str(self.root), base_facts("zagros-dev"))
        with patch.object(esf, "external_infra_entry",
                          return_value={"peer": "x", "exact": ["ZAGROS-DEV-EXTERNAL-INFRA", "ZAGROS-DEV-DEVELOPER"],
                                        "owner": "o", "reason": "r", "expires": "2026-12-31"}):
            with self.assertRaises(esf.Stop):
                esf.write_membership_entry(str(self.root), self.st, f, TODAY)
        self.assertFalse(self.mfile.exists())  # a fragment it created is removed again
        self.mfile.write_text("version: 1\nentries: []\n")
        before = self.mfile.read_text()
        with patch.object(esf, "external_infra_entry",
                          return_value={"peer": "x", "exact": ["ZAGROS-DEV-EXTERNAL-INFRA", "ZAGROS-DEV-DEVELOPER"],
                                        "owner": "o", "reason": "r", "expires": "2026-12-31"}):
            with self.assertRaises(esf.Stop):
                esf.write_membership_entry(str(self.root), self.st, f, TODAY)
        self.assertEqual(self.mfile.read_text(), before)

    def test_tier_on_governed_scope_writes_nothing_live(self):
        f = esf.governance(str(self.root), base_facts("zagros-dev"))
        with patch.object(esf, "nb") as nb:
            esf.do_tier(None, dict(self.st), f)
        nb.assert_not_called()

    def test_tier_on_ungoverned_scope_still_writes_live(self):
        f = esf.governance(str(self.root), base_facts("zagros-prod"))
        calls = []

        def fake_nb(facts, path, method="GET", body=None):
            calls.append((method, path))
            if path == "/groups":
                return [{"id": "g1", "name": f["external_infra"]}]
            return {"id": "g1", "name": f["external_infra"], "peers": []}

        with patch.object(esf, "nb", side_effect=fake_nb):
            esf.do_tier(None, dict(self.st, peer_id="p1", remove=[]), f)
        self.assertIn(("PUT", "/groups/g1"), calls)


class ResumeTest(unittest.TestCase):
    """A preview must leave a saved apply worktree usable by the real resume."""

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.git("init", "--quiet")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null",
                 "commit", "--quiet", "--allow-empty", "-m", "fixture")
        self.tree = self.root / "apply"
        self.branch = "esf-apply-123"
        self.git("worktree", "add", "--quiet", "-b", self.branch, str(self.tree))
        declaration = self.tree / "ops/ansible/group_vars/fixture_hosts/external-service-fixture.yml"
        declaration.parent.mkdir(parents=True)
        declaration.write_text("external_service_fixture:\n  enabled: true\n")
        self.note = self.tree / "operator-note.txt"
        self.note.write_text("untracked work to preserve\n")
        self.state_dir = self.root / "state"
        self.state_dir.mkdir()
        self.checkpoint = self.state_dir / "123.json"
        self.checkpoint.write_text(json.dumps({
            "pr": 123, "group": "fixture_hosts", "stem": "fixture", "name": "fixture",
            "peer_name": "fixture-peer", "hostnames": ["fixture"], "tls": "edge",
            "apply_tree": str(self.tree), "done": esf.STEPS[:8],
        }))
        self.facts = {
            "site_id": "fixture", "zone": "example.invalid", "edge_ip": "192.0.2.2",
            "external_infra": "FIXTURE-EXTERNAL-INFRA", "smtp_from": "fixture@example.invalid",
            "smtp_host": "example.invalid", "smtp_port": 587,
        }

    def git(self, *args):
        return subprocess.run(["git", "-C", str(self.repo), *args], check=True,
                              capture_output=True, text=True).stdout

    def resume(self, dry_run, apply=None, start=None, pull=None):
        argv = ["external-service-finish", "123", "--email", "fixture@example.invalid"]
        if dry_run:
            argv.append("--dry-run")
        if start:
            argv += ["--from", start]
        with contextlib.ExitStack() as stack:
            for name, value in (("REPO", str(self.repo)), ("STATE_DIR", str(self.state_dir)),
                                ("DRY", False)):
                stack.enter_context(patch.object(esf, name, value))
            stack.enter_context(patch.object(sys, "argv", argv))
            stack.enter_context(patch.object(esf, "site_facts", return_value=self.facts))
            stack.enter_context(patch.object(esf, "transport", return_value=("mesh", [])))
            declaration = base64.b64encode(b"external_service_fixture:\n  enabled: true\n").decode()
            stack.enter_context(patch.object(esf, "gh_json", side_effect=lambda args:
                {"state": "MERGED", "headRefOid": "fixture-head"} if args[0] == "pr"
                else {"content": declaration}))
            if pull is not None:
                stack.enter_context(patch.object(esf, "do_pull", side_effect=pull))
                stack.enter_context(patch.object(esf, "do_tier"))
            if apply is not None:
                stack.enter_context(patch.object(esf, "do_apply", side_effect=apply))
                stack.enter_context(patch.object(esf, "do_verify"))
                stack.enter_context(patch.object(esf, "do_email"))
            output = stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            return esf.main(), output.getvalue()

    def test_dry_run_preserves_saved_worktree_then_real_resume_cleans_up(self):
        checkpoint = self.checkpoint.read_bytes()
        branch_tip = self.git("rev-parse", self.branch)
        worktrees = self.git("worktree", "list", "--porcelain")
        code, output = self.resume(dry_run=True)
        self.assertEqual(code, 0)
        self.assertIn("Dry run complete.", output)
        self.assertEqual(self.checkpoint.read_bytes(), checkpoint)
        self.assertTrue(self.tree.is_dir())
        self.assertEqual(self.note.read_text(), "untracked work to preserve\n")
        self.assertEqual(self.git("rev-parse", self.branch), branch_tip)
        self.assertEqual(self.git("worktree", "list", "--porcelain"), worktrees)

        applied = []

        def apply(a, st, facts):
            self.assertFalse(esf.DRY)
            self.assertEqual(st["apply_tree"], str(self.tree))
            self.assertEqual(self.git("-C", st["apply_tree"], "rev-parse", "HEAD"), branch_tip)
            self.assertEqual(self.note.read_text(), "untracked work to preserve\n")
            applied.append(st["apply_tree"])

        code, _ = self.resume(dry_run=False, apply=apply)
        self.assertEqual(code, 0)
        self.assertEqual(applied, [str(self.tree)])
        self.assertFalse(self.tree.exists())
        self.assertNotIn(self.branch, self.git("branch", "--format=%(refname:short)").splitlines())
        self.assertFalse(self.checkpoint.exists())

    def test_stopped_resume_retains_worktree_and_checkpoint(self):
        checkpoint = self.checkpoint.read_bytes()

        def stop(a, st, facts):
            raise esf.Stop("synthetic apply failure")

        code, output = self.resume(dry_run=False, apply=stop)
        self.assertEqual(code, 1)
        self.assertIn("synthetic apply failure", output)
        self.assertTrue(self.tree.is_dir())
        self.assertEqual(self.note.read_text(), "untracked work to preserve\n")
        self.assertEqual(self.checkpoint.read_bytes(), checkpoint)
        self.assertIn(self.branch, self.git("branch", "--format=%(refname:short)").splitlines())

    def test_from_pull_refreshes_stale_declaration_before_mutations(self):
        path = self.tree / "ops/ansible/group_vars/fixture_hosts/external-service-fixture.yml"
        path.write_text("external_service_fixture: {enabled: false}\n")
        refreshed = []
        def pull(*args):
            path.write_text("external_service_fixture:\n  enabled: true\n")
            refreshed.append(True)
        def apply(*args):
            self.assertTrue(refreshed)
            self.assertIn("enabled: true", path.read_text())
        code, _ = self.resume(dry_run=False, start="pull", pull=pull, apply=apply)
        self.assertEqual(code, 0)
        self.assertEqual(refreshed, [True])


class ReviewTest(unittest.TestCase):
    """The review step stops waiting when the PR leaves the open state."""

    def setUp(self):
        self.st = {"pr": 1234, "url": "https://github.com/usareur-af/goatcore/pull/1234"}

    def review(self, *states):
        views = iter(states)
        with patch.object(esf, "gh_json", side_effect=lambda args: next(views)), \
                patch.object(esf, "run") as run, patch("time.sleep"):
            run.return_value.returncode = 0
            esf.do_review(None, self.st, {})
        return run

    def test_merged_while_waiting_moves_on(self):
        """A merge that bypassed review leaves REVIEW_REQUIRED forever; the state says it landed."""
        run = self.review({"state": "OPEN", "reviewDecision": "REVIEW_REQUIRED"},
                          {"state": "MERGED", "reviewDecision": "REVIEW_REQUIRED"})
        self.assertTrue(self.st["merged"])
        run.assert_not_called()

    def test_closed_stops(self):
        with self.assertRaises(esf.Stop):
            self.review({"state": "CLOSED", "reviewDecision": "REVIEW_REQUIRED"})

    def test_approved_then_watches_checks(self):
        run = self.review({"state": "OPEN", "reviewDecision": "REVIEW_REQUIRED"},
                          {"state": "OPEN", "reviewDecision": "APPROVED"})
        self.assertNotIn("merged", self.st)
        self.assertEqual(run.call_args.args[0][:3], ["gh", "pr", "checks"])


class StateCompletionTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.state = Path(tmp.name) / "1234.json"
        patches = ExitStack()
        self.addCleanup(patches.close)
        patches.enter_context(patch.object(esf, "STATE_DIR", tmp.name))
        patches.enter_context(patch.object(esf, "DRY", False))
        patches.enter_context(patch.object(esf, "STEPS", ["check", "email"]))
        patches.enter_context(patch.object(sys, "argv", ["external-service-finish.py", "1234"]))
        patches.enter_context(patch.object(sys, "stdout", io.StringIO()))
        self.check = patches.enter_context(patch.object(esf, "do_check"))
        self.email = patches.enter_context(patch.object(esf, "do_email"))

    def test_state_close_failure_stops_before_next_action(self):
        class CloseFailure(io.StringIO):
            def __exit__(self, *exc):
                super().__exit__(*exc)
                raise OSError("injected state close failure")

        handle = CloseFailure()
        self.addCleanup(handle.close)
        with patch.object(esf, "open", return_value=handle, create=True):
            with self.assertRaisesRegex(OSError, "injected state close failure"):
                esf.main()
        self.check.assert_called_once()
        self.email.assert_not_called()
        self.assertTrue(handle.closed)

    def test_resume_skips_completed_action_and_removes_finished_state(self):
        self.state.write_text(json.dumps({"done": ["check"]}))
        self.assertEqual(esf.main(), 0)
        self.check.assert_not_called()
        self.email.assert_called_once()
        self.assertFalse(self.state.exists())

    def test_dry_run_preserves_existing_state(self):
        before = json.dumps({"done": []})
        self.state.write_text(before)
        with patch.object(sys, "argv", ["external-service-finish.py", "1234", "--dry-run"]):
            self.assertEqual(esf.main(), 0)
        self.check.assert_called_once()
        self.email.assert_called_once()
        self.assertEqual(self.state.read_text(), before)


class ArchiveGuardTest(unittest.TestCase):
    def test_real_guard_refuses_invalid_resume_before_any_handler(self):
        for response in ("closed", "disabled", "missing", "malformed", "unavailable"):
            with self.subTest(response=response), tempfile.TemporaryDirectory() as tmp, ExitStack() as patches:
                Path(tmp, "1.json").write_text(json.dumps({
                    "pr": 1, "group": "fixture_hosts", "stem": "demo", "done": esf.STEPS[:8],
                }))
                patches.enter_context(patch.object(esf, "STATE_DIR", tmp))
                patches.enter_context(patch.object(sys, "argv", ["finish", "1"]))
                def source(args):
                    if response == "unavailable":
                        raise esf.Stop("synthetic GitHub read failure")
                    if args[0] == "pr":
                        return {"state": "CLOSED" if response == "closed" else "MERGED", "headRefOid": "head"}
                    body = {"disabled": "external_service_demo: {enabled: false}",
                            "missing": "{}", "malformed": "[invalid yaml"}[response]
                    return {"content": base64.b64encode(body.encode()).decode()}
                patches.enter_context(patch.object(esf, "gh_json", side_effect=source))
                handlers = [patches.enter_context(patch.object(esf, "do_" + name)) for name in esf.STEPS]
                self.assertEqual(esf.main(), 1)
                for handler in handlers:
                    handler.assert_not_called()

    def test_saved_apply_bytes_must_match_enabled_main(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp, "ops/ansible/group_vars/fixture_hosts/external-service-demo.yml")
            path.parent.mkdir(parents=True)
            body = b"external_service_demo: {enabled: true}\n"
            state = {"pr": 1, "group": "fixture_hosts", "stem": "demo", "apply_tree": tmp, "done": ["pull"]}
            def source(args):
                return ({"state": "MERGED", "headRefOid": "head"} if args[0] == "pr"
                        else {"content": base64.b64encode(body).decode()})
            with patch.object(esf, "gh_json", side_effect=source):
                path.write_bytes(body)
                esf.require_present_declaration(state)
                path.write_text("external_service_demo: {enabled: true, mesh_ip: changed}\n")
                with self.assertRaisesRegex(esf.Stop, "differs from current main"):
                    esf.require_present_declaration(state)

    def test_merge_is_bound_to_checked_head(self):
        state = {"pr": 1, "url": "fixture", "checked_head": "checked-enabled-head"}
        def merge(command, **kwargs):
            self.assertEqual(command[-2:], ["--match-head-commit", "checked-enabled-head"])
            raise esf.Stop("GitHub refused changed head")
        with patch.object(esf, "DRY", False), \
             patch.object(esf, "gh_json", return_value={"state": "OPEN", "reviewDecision": "APPROVED"}), \
             patch.object(esf, "run", side_effect=merge):
            with self.assertRaisesRegex(esf.Stop, "changed head"):
                esf.do_merge(None, state, {})
        self.assertNotIn("merged", state)

    def test_withdrawal_during_recipient_prompt_prevents_smtp(self):
        from types import SimpleNamespace
        state = {"name": "fixture"}
        facts = {"site_id": "fixture", "smtp_from": "fixture@example.invalid"}
        with patch.object(esf, "DRY", False), \
             patch.object(esf, "linked_email", return_value=("fixture", None, "unlinked")), \
             patch.object(esf, "ask", return_value="recipient@example.invalid"), \
             patch.object(esf, "confirm", return_value=True), \
             patch.object(esf, "config_text", return_value="fixture configuration"), \
             patch.object(esf, "require_present_declaration", side_effect=esf.Stop("withdrawn during prompt")), \
             patch.object(esf.smtplib, "SMTP") as smtp:
            with self.assertRaisesRegex(esf.Stop, "withdrawn during prompt"):
                esf.do_email(SimpleNamespace(email=None), state, facts)
            smtp.assert_not_called()

    def test_current_source_state(self):
        import base64
        for state in ("OPEN", "MERGED"):
            for enabled in (True, False):
                with self.subTest(state=state, enabled=enabled):
                    body = "external_service_demo: {enabled: %s}" % str(enabled).lower()
                    replies = [{"state": state, "headRefOid": "abc123"},
                               {"content": base64.b64encode(body.encode()).decode()}]
                    with patch.object(esf, "gh_json", side_effect=replies) as gh:
                        st = {"pr": 1, "group": "test_hosts", "stem": "demo"}
                        if enabled:
                            esf.require_present_declaration(st)
                        else:
                            with self.assertRaises(esf.Stop):
                                esf.require_present_declaration(st)
                        ref = "main" if state == "MERGED" else "abc123"
                        self.assertTrue(gh.call_args.args[0][1].endswith("?ref=" + ref))

    def test_withdrawn_during_review_stops_before_merge(self):
        with tempfile.TemporaryDirectory() as tmp, ExitStack() as patches:
            patches.enter_context(patch.object(esf, "STATE_DIR", tmp))
            patches.enter_context(patch.object(sys, "argv", ["finish", "1"]))
            patches.enter_context(patch.object(esf, "site_facts", return_value={}))
            patches.enter_context(patch.object(esf, "check_site"))
            enabled = [True]

            def guard(_state):
                if not enabled[0]:
                    raise esf.Stop("withdrawn during review")

            patches.enter_context(patch.object(esf, "require_present_declaration", side_effect=guard))
            handlers = {name: patches.enter_context(patch.object(esf, "do_" + name))
                        for name in esf.STEPS}
            handlers["read"].side_effect = lambda args, state: state.update(group="fixture_hosts")
            handlers["review"].side_effect = lambda *args: enabled.__setitem__(0, False)
            self.assertEqual(esf.main(), 1)
            handlers["review"].assert_called_once()
            for name in ("merge", "tier", "apply", "email"):
                handlers[name].assert_not_called()

    def test_resume_refuses_before_handlers(self):
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "1.json").write_text(json.dumps({"done": ["check", "read"]}))
            with patch.object(esf, "STATE_DIR", tmp), \
                 patch.object(sys, "argv", ["finish", "1"]), \
                 patch.object(esf, "require_present_declaration", side_effect=esf.Stop("archived")), \
                 patch.object(esf, "do_peer") as peer, \
                 patch.object(esf, "do_tier") as tier, \
                 patch.object(esf, "do_email") as email:
                self.assertNotEqual(esf.main(), 0)
                peer.assert_not_called()
                tier.assert_not_called()
                email.assert_not_called()


if __name__ == "__main__":
    unittest.main(verbosity=1)
