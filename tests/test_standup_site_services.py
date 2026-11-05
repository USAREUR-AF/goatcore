#!/usr/bin/env python3
"""goat-standup.sh phase 12 deploys the site services a goatnet enables.

site-services.yml imports isitup-deploy.yml and id-admin-console-deploy.yml, but
the driver runs it only on the LAN profile and only for its lan-dns, traefik and
stun tags, so no phase deployed either service. A site that enables them, lists
them in isitup_services and routes them in Traefik then fails the phase-13
`declared-service-actually-deployed` row (the console answered 502).

Planned and run from a fixture tree: no host, no AWS, ansible stubbed.
"""
import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SITE = "svctest"
GROUP = f"{SITE}_hosts"
CONSOLE = "id-admin console (single-homed admin surface; opt-in via id_admin_console_enabled)"
ISITUP = "isitup status page (opt-in via isitup_enabled)"


def plan(repo, profile):
    env = {"PATH": os.environ["PATH"], "HOME": str(repo)}
    r = subprocess.run(["bash", str(repo / "ops/scripts/goat-standup.sh"), SITE, "--plan",
                        "--profile", profile, "--from", "12", "--to", "12"],
                       capture_output=True, text=True, env=env, timeout=120)
    if r.returncode != 0:
        raise AssertionError(r.stdout + r.stderr)
    return r.stdout


def steps(out):
    """[(label, command)] for every phase-12 step, in plan order."""
    return re.findall(r"^=== Phase 12 \[[A-Z]+\] (.*) ===\n    (.*)\n", out, re.M)


def command(out, label):
    for lab, cmd in steps(out):
        if lab == label:
            return cmd
    raise AssertionError(f"no phase-12 step {label!r} in the plan:\n{out}")


class Fixture:
    def __init__(self, vars_text):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name)
        scripts = self.repo / "ops/scripts"
        scripts.mkdir(parents=True)
        shutil.copy(ROOT / "ops/scripts/goat-standup.sh", scripts / "goat-standup.sh")
        gv = self.repo / "ops/ansible/group_vars" / GROUP
        gv.mkdir(parents=True)
        (gv / "vars.yml").write_text(vars_text)
        self.calls = self.repo / "calls"
        self.bin = self.repo / "bin"
        self.bin.mkdir()
        (scripts / "with-aws").write_text('#!/usr/bin/env bash\nshift 2\nexec "$@"\n')
        (scripts / "ansible-preflight.sh").write_text(
            f'#!/usr/bin/env bash\nprintf "%s\\n" "$*" >> "{self.calls}"\nexit "${{STUB_RC:-0}}"\n')
        for f in ("with-aws", "ansible-preflight.sh"):
            (scripts / f).chmod(0o755)

    def run(self, cmd, rc=0):
        env = {"PATH": os.environ["PATH"], "HOME": str(self.repo), "STUB_RC": str(rc)}
        r = subprocess.run(["bash", "-c", cmd], capture_output=True, text=True, env=env, timeout=60)
        called = self.calls.read_text().splitlines() if self.calls.exists() else []
        return r, called

    def close(self):
        self.tmp.cleanup()


class Plan(unittest.TestCase):
    def test_both_profiles_deploy_the_console_and_isitup_after_the_portal(self):
        for profile in ("internet", "lan"):
            out = plan(ROOT, profile)
            labels = [lab for lab, _ in steps(out)]
            self.assertIn(CONSOLE, labels, profile)
            self.assertIn(ISITUP, labels, profile)
            portal = labels.index("portal (registry, certificate minting, grant rows)")
            self.assertLess(portal, labels.index(CONSOLE), "the console reads the portal")
            self.assertLess(labels.index(CONSOLE), labels.index(ISITUP),
                            "the status page comes after the surfaces it lists")
            console = command(out, CONSOLE)
            self.assertIn("id-admin-console-deploy.yml", console)
            self.assertIn(f"-e target={GROUP}", console)
            self.assertIn("isitup-deploy.yml", command(out, ISITUP))

    def test_both_are_single_homed_on_the_founder_mgmt_host(self):
        out = plan(ROOT, "internet")
        for label in (CONSOLE, ISITUP):
            self.assertIn(f"-l 'mgmt-{SITE}-01*'", command(out, label))

    def test_both_are_in_the_phase_12_graph_after_the_portal(self):
        import yaml
        dag = yaml.safe_load((ROOT / "ops/goatbuilder/standup-dag.yaml").read_text())["steps"]
        by_label = {s["label"]: s for s in dag}
        for label in (CONSOLE, ISITUP):
            hits = [s for lab, s in by_label.items() if label.startswith(lab)]
            self.assertEqual(len(hits), 1, f"{label!r} matches {len(hits)} dag steps")
            self.assertIn("portal", hits[0]["needs"])


class Gate(unittest.TestCase):
    """The step runs the play only when the site enables the service, and a failed
    play fails the step (it is not swallowed into a skip)."""

    def step(self, vars_text, label, rc=0):
        fx = Fixture(vars_text)
        self.addCleanup(fx.close)
        cmd = command(plan(fx.repo, "internet"), label)
        return fx.run(cmd, rc)

    def test_enabled_runs_the_play(self):
        for label, flag, play in ((CONSOLE, "id_admin_console_enabled", "id-admin-console-deploy.yml"),
                                  (ISITUP, "isitup_enabled", "isitup-deploy.yml")):
            r, called = self.step(f"site_id: x\n{flag}: true\n", label)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertEqual(len(called), 1, called)
            self.assertIn(play, called[0])
            self.assertIn(f"-e target={GROUP}", called[0])

    def test_disabled_or_commented_out_skips_without_running(self):
        for text in ("id_admin_console_enabled: false\nisitup_enabled: false\n",
                     "# id_admin_console_enabled: true\n# isitup_enabled: true\n",
                     "other: 1\n"):
            for label in (CONSOLE, ISITUP):
                r, called = self.step(text, label)
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertEqual(called, [], (text, label))
                self.assertIn("skipping", r.stdout)

    def test_a_failed_play_fails_the_step(self):
        for label, flag in ((CONSOLE, "id_admin_console_enabled"), (ISITUP, "isitup_enabled")):
            r, called = self.step(f"{flag}: true\n", label, rc=7)
            self.assertEqual(r.returncode, 7, (label, r.stdout))
            self.assertNotIn("skipping", r.stdout)


class RequiredInputs(unittest.TestCase):
    """standup-required-inputs.py reads the same guard: a site that does not enable
    the service is not asked for its secrets."""

    def plays(self, vars_text, label):
        import importlib.util
        fx = Fixture(vars_text)
        self.addCleanup(fx.close)
        cmd = command(plan(fx.repo, "internet"), label)
        spec = importlib.util.spec_from_file_location("sri", ROOT / "ops/scripts/standup-required-inputs.py")
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        mod.ANS = fx.repo / "ops/ansible"
        # plays_in only reports plays that exist in the tree it reads
        for p in ("id-admin-console-deploy.yml", "isitup-deploy.yml"):
            (mod.ANS / p).write_text("---\n")
        return [p for p, *_ in mod.plays_in(cmd, GROUP)]

    def test_the_guard_is_honoured(self):
        self.assertEqual(self.plays("site_id: x\nid_admin_console_enabled: true\n", CONSOLE), ["id-admin-console-deploy.yml"])
        self.assertEqual(self.plays("site_id: x\nisitup_enabled: true\n", ISITUP), ["isitup-deploy.yml"])
        self.assertEqual(self.plays("# id_admin_console_enabled: true\n", CONSOLE), [])
        self.assertEqual(self.plays("isitup_enabled: false\n", ISITUP), [])


if __name__ == "__main__":
    unittest.main()
