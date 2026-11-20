#!/usr/bin/env python3
"""ops/scripts/standup-phase-run: a phase whose $(printenv NAME) is empty refuses and
names NAME; a value store's values reach the phase without being exported by hand."""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

RUN = Path(__file__).resolve().parents[1] / "ops/scripts/standup-phase-run"
CMD = 'test -n "$(printenv GALAXY_PG_PASS)" && echo ran'


def run(cmd, store, extra=None, check=False):
    env = {"PATH": os.environ["PATH"], "HOME": tempfile.mkdtemp(), "GOAT_SECRET_STORE": store}
    env.update(extra or {})
    args = [str(RUN)] + (["--check"] if check else []) + [cmd]
    return subprocess.run(args, capture_output=True, text=True, env=env)


class Refusal(unittest.TestCase):
    def test_an_empty_name_refuses_and_is_named(self):
        r = run(CMD, "env")
        self.assertEqual(r.returncode, 3, r.stderr)
        self.assertIn("GALAXY_PG_PASS", r.stderr)
        self.assertNotIn("ran", r.stdout, "the phase never ran")

    def test_every_empty_name_is_named_at_once(self):
        r = run('x="$(printenv A_ONE)" y="$(printenv A_TWO)"; true', "env", {"A_ONE": ""})
        self.assertEqual(r.returncode, 3)
        self.assertIn("A_ONE A_TWO", r.stderr)

    def test_a_reference_store_refuses_too(self):
        self.assertEqual(run(CMD, "1password").returncode, 3)

    def test_a_set_name_runs_the_phase(self):
        r = run(CMD, "env", {"GALAXY_PG_PASS": "s3cret-value"})
        self.assertEqual((r.returncode, r.stdout.strip()), (0, "ran"), r.stderr)
        self.assertNotIn("s3cret-value", r.stdout + r.stderr)

    def test_a_name_a_play_reads_from_the_environment_is_checked_too(self):
        # A standup line passes a secret through the environment, not the command
        # line: the play reads it with lookup("env", ...). An empty one refuses
        # the same way, and a set one runs without the value on any command line.
        cmd = """printenv GALAXY_PG_PASS >/dev/null && echo ran -e 'postgres_superuser_password={{ lookup("env", "GALAXY_PG_PASS") }}' -e 'k={{ lookup("env", "GALAXY_NB_ENCKEY") }}'"""
        r = run(cmd, "env", {"GALAXY_NB_ENCKEY": "k"})
        self.assertEqual(r.returncode, 3, r.stderr)
        self.assertIn("reads GALAXY_PG_PASS,", r.stderr)
        self.assertNotIn("ran", r.stdout)
        r = run(cmd, "env", {"GALAXY_PG_PASS": "s3cret-value", "GALAXY_NB_ENCKEY": "k"})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertNotIn("s3cret-value", r.stdout + r.stderr)

    def test_check_only_never_runs_the_command(self):
        r = run("echo ran", "env", check=True)
        self.assertEqual((r.returncode, r.stdout), (0, ""))

    def test_the_phase_status_is_kept(self):
        self.assertEqual(run("exit 7", "env").returncode, 7)


class UnsetVariable(unittest.TestCase):
    """A phase line that expands $NAME or ${NAME} with NAME unset would stop on
    bash -u's own 'NAME: unbound variable' before its program starts. It refuses
    first, after the store is loaded, with GB-369 and the name (a standup line
    with no administrator declared carries "$OWNER_EMAIL")."""

    OWNER = 'echo -e bootstrap_owner_email="$OWNER_EMAIL" && echo ran'

    def test_an_unset_name_refuses_with_gb_369_and_is_named(self):
        r = run(self.OWNER, "env")
        self.assertEqual(r.returncode, 3, r.stderr)
        self.assertIn("GB-369", r.stderr)
        self.assertIn("OWNER_EMAIL", r.stderr)
        self.assertNotIn("ran", r.stdout, "the phase never ran")
        self.assertNotIn("unbound variable", r.stderr, "refused before bash met it")

    def test_check_only_refuses_it_too(self):
        r = run(self.OWNER, "env", check=True)
        self.assertEqual(r.returncode, 3, r.stderr)
        self.assertIn("OWNER_EMAIL", r.stderr)

    def test_a_set_name_runs_and_an_empty_one_is_bash_s_business(self):
        r = run(self.OWNER, "env", {"OWNER_EMAIL": "ops@example.org"})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("ran", r.stdout)
        r = run(self.OWNER, "env", {"OWNER_EMAIL": ""})
        self.assertEqual(r.returncode, 0, "bash -u stops only on an unset name: " + r.stderr)

    def test_the_store_supplies_it_before_the_check(self):
        d = Path(tempfile.mkdtemp())
        f = d / "secrets.env"
        f.write_text("OWNER_EMAIL=ops@example.org\n")
        f.chmod(0o600)
        r = run(self.OWNER, "env-file", {"GOAT_SECRET_ENV_FILE": str(f)})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("ran", r.stdout)

    def test_quoted_defaulted_and_assigned_names_are_not_read(self):
        cmd = """bash -c 'echo "$LATER"' && echo ${WITH_DEFAULT:-x} \\$NOT_A_REF && VF=$(mktemp) && echo "$VF" && for h in a; do echo "$h"; done && echo ran"""
        r = run(cmd, "env")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("ran", r.stdout)


class ValueStore(unittest.TestCase):
    def test_phase_3_values_reach_a_later_phase_from_the_env_file(self):
        d = Path(tempfile.mkdtemp())
        f = d / "secrets.env"
        f.write_text("GALAXY_PG_PASS=from-the-file\n")
        f.chmod(0o600)
        r = run(CMD, "env-file", {"GOAT_SECRET_ENV_FILE": str(f)})
        self.assertEqual((r.returncode, r.stdout.strip()), (0, "ran"), r.stderr)
        self.assertNotIn("from-the-file", r.stdout + r.stderr, "no value is printed")

    def test_an_env_file_without_the_name_still_refuses(self):
        d = Path(tempfile.mkdtemp())
        f = d / "secrets.env"
        f.write_text("OTHER=x\n")
        r = run(CMD, "env-file", {"GOAT_SECRET_ENV_FILE": str(f)})
        self.assertEqual(r.returncode, 3)
        self.assertIn("GALAXY_PG_PASS", r.stderr)


KEEP = Path(__file__).resolve().parents[1] / "ops/scripts/standup-keep-secret"


class KeepSecret(unittest.TestCase):
    """A value a step mints (the Forgejo mirror token) reaches the phases after it: under a
    value store it is written to the store; under a reference store nothing changes."""

    def keep(self, store, extra):
        env = {"PATH": os.environ["PATH"], "HOME": tempfile.mkdtemp(), "GOAT_SECRET_STORE": store}
        env.update(extra)
        return subprocess.run([str(KEEP), "FORGEJO_PUBSUB_TOKEN"], capture_output=True, text=True, env=env)

    def test_a_value_store_keeps_it_and_the_next_phase_reads_it(self):
        f = Path(tempfile.mkdtemp()) / "secrets.env"
        f.write_text("OTHER=x\n")
        f.chmod(0o600)
        r = self.keep("env-file", {"GOAT_SECRET_ENV_FILE": str(f), "FORGEJO_PUBSUB_TOKEN": "minted-token"})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertNotIn("minted-token", r.stdout + r.stderr, "never printed")
        nxt = run('test -n "$(printenv FORGEJO_PUBSUB_TOKEN)" && echo ran', "env-file", {"GOAT_SECRET_ENV_FILE": str(f)})
        self.assertEqual(nxt.stdout.strip(), "ran", nxt.stderr)
        again = self.keep("env-file", {"GOAT_SECRET_ENV_FILE": str(f), "FORGEJO_PUBSUB_TOKEN": "a-second-token"})
        self.assertIn("kept", again.stdout, "an existing value is never rotated")

    def test_a_reference_store_is_left_as_it_was(self):
        r = self.keep("1password", {"FORGEJO_PUBSUB_TOKEN": "minted-token"})
        self.assertEqual(r.returncode, 0)
        self.assertIn("not a value store", r.stdout)

    def test_an_empty_token_is_refused(self):
        f = Path(tempfile.mkdtemp()) / "secrets.env"
        self.assertEqual(self.keep("env-file", {"GOAT_SECRET_ENV_FILE": str(f)}).returncode, 1)
        self.assertFalse(f.exists(), "nothing written")


ROOT = Path(__file__).resolve().parents[1]
STUB_RENDER = r"""#!/usr/bin/env python3
# Stands in for goatlite-render.py: writes the group the goatnet's plays template from.
import os, sys
repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
if os.environ.get("STUB_RENDER_FAILS"):
    sys.exit("goatlite-render: refusing: a file you edited differs from the render")
d = os.path.join(repo, "ops/ansible/group_vars/cleanroom_hosts")
os.makedirs(d, exist_ok=True)
with open(os.path.join(d, "vars.yml"), "w") as fh:
    fh.write("pg: \"{{ lookup('env', 'CLEANROOM_PG_PASS') }}\"\n"
             "cookie: \"{{ lookup('env', 'CLEANROOM_UI_COOKIE_SECRET') }}\"\n")
print("rendered cleanroom")
"""


class GoatliteFirstRun(unittest.TestCase):
    """LP-007: phase 1 of a goatlite renders the config surface, creates the env-file
    store itself, and only then runs what reads the store. A checkout with neither the
    store nor the rendered group needs no manual step; a re-run keeps every value."""

    def setUp(self):
        self.repo = Path(tempfile.mkdtemp())
        for rel in ("ops/scripts/standup-phase-run", "ops/secrets/posture.py", "ops/scripts/deploy-secrets-needed.py"):
            (self.repo / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / rel, self.repo / rel)
        render = self.repo / "ops/scripts/goatlite-render.py"
        render.write_text(STUB_RENDER)
        render.chmod(0o755)
        (self.repo / "goatlite.yaml").write_text(
            "schema: goatbuilder/v1\npreset: lite\nlite:\n  name: cleanroom   # the goatnet\n  host: 192.0.2.50\n")
        (self.repo / ".gitignore").write_text(".envrc.local.d/\n")
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True)
        self.store = self.repo / ".envrc.local.d/secrets.env"
        r = self.repo
        # Phase 1's lines as the plan prints them (standup.rs, goat-standup.sh).
        self.render = f"python3 '{r}/ops/scripts/goatlite-render.py' --site cleanroom"
        self.init = f"python3 '{r}/ops/secrets/posture.py' init --if-env-file --env cleanroom"
        self.consumer = 'test -n "$(printenv CLEANROOM_PG_PASS)" && test -n "$(printenv CLEANROOM_UI_COOKIE_SECRET)" && echo consumed'

    def phase(self, cmd, extra=None):
        env = {"PATH": os.environ["PATH"], "HOME": tempfile.mkdtemp()}
        env.update(extra or {})
        return subprocess.run([str(self.repo / "ops/scripts/standup-phase-run"), cmd],
                              capture_output=True, text=True, env=env, cwd=self.repo)

    def phase_one(self):
        out = []
        for cmd in (self.render, self.init, self.consumer):
            r = self.phase(cmd)
            out.append(r)
            if r.returncode:
                break
        return out

    def test_an_unseeded_checkout_renders_creates_the_store_then_reads_it(self):
        self.assertFalse(self.store.exists())
        self.assertFalse((self.repo / "ops/ansible/group_vars/cleanroom_hosts").exists())
        runs = self.phase_one()
        self.assertEqual([r.returncode for r in runs], [0, 0, 0], [r.stderr for r in runs])
        self.assertEqual(runs[2].stdout.strip(), "consumed")
        self.assertEqual(oct(self.store.stat().st_mode & 0o777), "0o600")
        values = dict(l.split("=", 1) for l in self.store.read_text().splitlines() if l and not l.startswith("#"))
        self.assertEqual(sorted(values), ["CLEANROOM_PG_PASS", "CLEANROOM_UI_COOKIE_SECRET"])
        printed = "".join(r.stdout + r.stderr for r in runs)
        for v in values.values():
            self.assertTrue(v)
            self.assertNotIn(v, printed, "no value is printed")

    def test_a_re_run_keeps_every_value(self):
        self.phase_one()
        before = self.store.read_bytes()
        runs = self.phase_one()
        self.assertEqual([r.returncode for r in runs], [0, 0, 0], [r.stderr for r in runs])
        self.assertEqual(self.store.read_bytes(), before, "a second phase 1 regenerates nothing")
        self.assertIn("2 kept", runs[1].stdout)

    def test_a_failing_render_still_fails_the_phase(self):
        r = self.phase(self.render, {"STUB_RENDER_FAILS": "1"})
        self.assertEqual(r.returncode, 1, r.stderr)
        self.assertIn("refusing", r.stderr)
        self.assertFalse(self.store.exists())

    def test_any_other_line_still_needs_the_store(self):
        r = self.phase("echo ran")
        self.assertEqual(r.returncode, 1)
        self.assertNotIn("ran", r.stdout)
        self.assertIn("goatbuilder standup cleanroom --to 1", r.stderr, "the refusal names the one command that fixes it")
        chained = self.phase(self.render + " && echo ran")
        self.assertEqual(chained.returncode, 1, "only the exact render line skips the store")

    def test_secrets_init_before_the_render_names_phase_1(self):
        env = {"PATH": os.environ["PATH"], "HOME": tempfile.mkdtemp()}
        r = subprocess.run(["python3", str(self.repo / "ops/secrets/posture.py"), "init", "--env", "cleanroom"],
                           capture_output=True, text=True, env=env)
        self.assertEqual(r.returncode, 1)
        self.assertIn("goatbuilder standup cleanroom --to 1", r.stderr)

    def test_another_store_creates_no_plaintext_file(self):
        self.phase(self.render)
        r = self.phase(self.init, {"GOAT_SECRET_STORE": "env"})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("nothing to create", r.stdout)
        self.assertFalse(self.store.exists())


if __name__ == "__main__":
    unittest.main()
