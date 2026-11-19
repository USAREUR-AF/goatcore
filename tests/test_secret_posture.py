"""The declared secrets posture (ADR 1204): one test per store, the injector, and the quickstart.

Run: python3 -m unittest tests/test_secret_posture.py -v
"""
import importlib.util
import os
import stat
import shutil
import subprocess
import tempfile
import textwrap
import unittest
import unittest.mock as mock

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
POSTURE = os.path.join(REPO, "ops/secrets/posture.py")
WITH_SECRETS = os.path.join(REPO, "ops/scripts/with-secrets")
IGNORED_DIR = os.path.join(REPO, ".envrc.local.d")  # gitignored by the repository


def load():
    spec = importlib.util.spec_from_file_location("posture", POSTURE)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


# The tests' environment: no posture override and no credential reference, whatever
# the shell running them carries.
CLEAN = {k: v for k, v in os.environ.items()
         if not k.startswith("GOAT_SECRET") and not v.startswith(("op://", "bao://"))}


class Base(unittest.TestCase):
    def setUp(self):
        self.p = load()
        os.makedirs(IGNORED_DIR, exist_ok=True)
        self.tmp = tempfile.mkdtemp(dir=IGNORED_DIR, prefix="test-posture-")
        self.cfg = os.path.join(self.tmp, "goatbuilder.yaml")

    def tearDown(self):
        subprocess.run(["rm", "-rf", self.tmp])

    def write(self, name, text):
        path = os.path.join(self.tmp, name)
        with open(path, "w") as fh:
            fh.write(textwrap.dedent(text))
        os.chmod(path, 0o600)
        return path

    def env(self, **kw):
        e = dict(CLEAN)
        e["GOAT_BUILDER_CONFIG"] = self.cfg
        e.update(kw)
        return mock.patch.dict(os.environ, e, clear=True)


class Declaration(Base):
    def test_default_is_the_store_that_needs_no_account(self):
        # A checkout that declares nothing resolves its own values rather than
        # pointing a stranger at this project's 1Password tenant.
        with self.env():
            self.assertEqual(self.p.store(), "env-file")

    def test_config_declares_and_env_overrides(self):
        self.write("goatbuilder.yaml", "components: {}\nsecrets:\n  store: env-file\n  env_file: x.env\npromotion: {}\n")
        with self.env():
            self.assertEqual(self.p.store(), "env-file")
            self.assertTrue(self.p.store_file().endswith("x.env"))
        with self.env(GOAT_SECRET_STORE="env"):
            self.assertEqual(self.p.store(), "env")

    def test_unknown_store_is_refused(self):
        with self.env(GOAT_SECRET_STORE="vaultish"):
            with self.assertRaises(self.p.PostureError):
                self.p.store()

    def test_this_tree_declares_the_store_that_needs_no_account(self):
        with mock.patch.dict(os.environ, CLEAN, clear=True):
            self.assertEqual(self.p.store(), "env-file")


class EnvFile(Base):
    def test_values_and_availability(self):
        f = self.write("s.env", "# comment\nexport A_SECRET='one'\nB_SECRET=two\nEMPTY=\n")
        with self.env(GOAT_SECRET_STORE="env-file", GOAT_SECRET_ENV_FILE=f):
            self.assertEqual(self.p.values(), {"A_SECRET": "one", "B_SECRET": "two", "EMPTY": ""})
            have = self.p.available()
            self.assertIn("A_SECRET", have)
            self.assertNotIn("EMPTY", have)

    def test_a_reference_in_the_shell_is_not_available_under_a_value_store(self):
        f = self.write("s.env", "A_SECRET=one\n")
        with self.env(GOAT_SECRET_STORE="env-file", GOAT_SECRET_ENV_FILE=f, B_SECRET="op://v/i/f"):
            self.assertNotIn("B_SECRET", self.p.available())
        with self.env(GOAT_SECRET_STORE="1password", B_SECRET="op://v/i/f"):
            self.assertIn("B_SECRET", self.p.available())

    def test_doctor_refuses_a_world_readable_file(self):
        # The doctor reads the file's mode; a group-readable mode is fed to it here
        # rather than written to disk.
        f = self.write("s.env", "A=1\n")
        real_stat = os.stat
        def group_readable(path, *a, **k):
            st = real_stat(path, *a, **k)
            if path == f:
                return os.stat_result((stat.S_IFREG | 0o640,) + tuple(st)[1:])
            return st
        with self.env(GOAT_SECRET_STORE="env-file", GOAT_SECRET_ENV_FILE=f), mock.patch.object(self.p.os, "stat", group_readable):
            rows = {r[1]: r for r in self.p.doctor()}
            self.assertFalse(rows["secrets:env-file-mode"][0])

    def test_doctor_refuses_a_file_git_would_commit(self):
        path = os.path.join(REPO, "ops/secrets/not-ignored-test.env")
        with open(path, "w") as fh:
            fh.write("A=1\n")
        os.chmod(path, 0o600)
        try:
            with self.env(GOAT_SECRET_STORE="env-file", GOAT_SECRET_ENV_FILE=path):
                rows = {r[1]: r for r in self.p.doctor()}
                self.assertFalse(rows["secrets:env-file-ignored"][0])
        finally:
            os.unlink(path)


class GitCrypt(Base):
    def test_locked_file_is_refused_and_unlocked_reads(self):
        locked = os.path.join(self.tmp, "locked.env")
        with open(locked, "wb") as fh:
            fh.write(b"\x00GITCRYPT\x00" + os.urandom(16))
        with self.env(GOAT_SECRET_STORE="git-crypt", GOAT_SECRET_GITCRYPT_FILE=locked):
            with self.assertRaises(self.p.PostureError):
                self.p.values()
            self.assertFalse([r for r in self.p.doctor() if r[1] == "secrets:git-crypt"][0][0])
        unlocked = self.write("unlocked.env", "C_SECRET=three\n")
        with self.env(GOAT_SECRET_STORE="git-crypt", GOAT_SECRET_GITCRYPT_FILE=unlocked):
            self.assertEqual(self.p.values(), {"C_SECRET": "three"})


class Sops(Base):
    def test_names_without_decrypting_and_values_through_sops(self):
        f = self.write("s.sops.env", "D_SECRET=ENC[AES256_GCM,data:x]\nsops_version=3.9.0\n")
        bindir = os.path.join(self.tmp, "bin")
        os.makedirs(bindir)
        fake = os.path.join(bindir, "sops")
        with open(fake, "w") as fh:
            fh.write("#!/bin/sh\necho D_SECRET=four\n")
        os.chmod(fake, 0o700)
        with self.env(GOAT_SECRET_STORE="sops", GOAT_SECRET_SOPS_FILE=f, PATH=bindir + os.pathsep + os.environ["PATH"]):
            self.assertIn("D_SECRET", self.p.available())
            self.assertNotIn("sops_version", self.p.available())
            self.assertEqual(self.p.values(), {"D_SECRET": "four"})


class OpenBao(Base):
    def test_doctor_names_each_missing_piece_of_the_store_contract(self):
        home = os.path.join(self.tmp, "home")
        os.makedirs(home)
        with self.env(GOAT_SECRET_STORE="openbao", HOME=home):
            names = {r[1] for r in self.p.doctor() if not r[0]}
            self.assertTrue({"secrets:openbao-addr", "secrets:openbao-cacert", "secrets:openbao-session"} <= names, names)

    def test_a_valid_token_reads_ok(self):
        bindir = os.path.join(self.tmp, "bin")
        os.makedirs(bindir)
        with open(os.path.join(bindir, "bao"), "w") as fh:
            fh.write('#!/bin/sh\n[ "$1 $2" = "token lookup" ] && echo "{}" && exit 0\nexit 9\n')
        os.chmod(os.path.join(bindir, "bao"), 0o700)
        ca = self.write("ca.crt", "-----BEGIN CERTIFICATE-----\n")
        with self.env(GOAT_SECRET_STORE="openbao", BAO_ADDR="https://10.0.0.1:8200", BAO_CACERT=ca, BAO_TOKEN="t",
                      PATH=bindir + os.pathsep + os.environ["PATH"]):
            rows = {r[1]: r for r in self.p.doctor()}
            self.assertTrue(rows["secrets:openbao-session"][0])
            self.assertNotIn("secrets:openbao-addr", rows)

    def test_the_injector_dispatches_to_the_store_run_script(self):
        # bao:// references go to the OpenBao store's injector, ops/secrets/openbao/run.
        r = subprocess.run([WITH_SECRETS, "true"], env=dict(CLEAN, GOAT_SECRET_STORE="openbao", G="bao://goat/x/y",
                           GOAT_BUILDER_CONFIG=self.cfg), capture_output=True, text=True)
        run = os.path.join(REPO, "ops/secrets/openbao/run")
        if os.path.exists(run):
            self.skipTest("the openbao injector is present; its own tests cover it")
        self.assertEqual(r.returncode, 127)
        self.assertIn("ops/secrets/openbao/run", r.stderr)


class Hints(Base):
    def test_every_store_tells_the_operator_what_to_do(self):
        for s in self.p.STORES:
            with self.env(GOAT_SECRET_STORE=s):
                text = " ".join(self.p.hint(["X_SECRET"]))
                self.assertTrue(text, s)
        # One sign-in shape per backend.
        with self.env(GOAT_SECRET_STORE="1password"):
            self.assertIn("sign in: eval $(op signin)", " ".join(self.p.hint(["X"])))
        with self.env(GOAT_SECRET_STORE="openbao"):
            self.assertIn("sign in: bao login -method=oidc", " ".join(self.p.hint(["X"])))
        with self.env(GOAT_SECRET_STORE="env-file"):
            self.assertIn("secrets init", " ".join(self.p.hint(["X"])))


FALSE_CMD = shutil.which("false") or "/usr/bin/false"


class NoCredentialReadWithoutAsking(Base):
    """`doctor` runs with GOAT_STORE_PROBE=0 unless --credentials is passed, so
    no store may read a session there: reading one unlocks a credential store,
    and on a Mac that is a biometric prompt on a read-only command."""

    def session_rows(self, **env):
        with self.env(**env):
            return {c: d for ok, c, d, _fix in self.p.doctor() if "session" in c}

    def test_a_store_that_is_not_probed_says_so_rather_than_passing(self):
        for store, extra in [
            ("1password", {"OP_ACCOUNT": "example.1password.com"}),
            ("openbao", {"BAO_ADDR": "https://bao.example", "BAO_TOKEN": "x"}),
            ("vaultwarden", {"BW_SESSION": "x", "GOAT_BW_CLI": FALSE_CMD}),
        ]:
            cli = {"1password": "op", "openbao": "bao"}.get(store)
            if cli and not shutil.which(cli):
                continue  # doctor reports a store's session only where that store's CLI is installed
            rows = self.session_rows(GOAT_SECRET_STORE=store, GOAT_STORE_PROBE="0", **extra)
            self.assertTrue(rows, f"{store}: no session row, so this test proved nothing")
            for check, detail in rows.items():
                self.assertTrue(
                    detail.startswith("not checked"),
                    f"{store}: {check} reads a credential with the probe off: {detail}",
                )

    def test_the_probe_runs_the_store_when_it_is_not_suppressed(self):
        # `false` stands in for the CLI: with the probe on it is run and its
        # failure is reported, which is how we know the probe is not skipped.
        rows = self.session_rows(GOAT_SECRET_STORE="vaultwarden", BW_SESSION="x", GOAT_BW_CLI=FALSE_CMD)
        self.assertTrue(rows, "the vaultwarden store reports a session row")
        self.assertFalse(
            any(d.startswith("not checked") for d in rows.values()),
            f"the probe was suppressed without being asked: {rows}",
        )


class Injector(Base):
    def run_with(self, env, cmd):
        return subprocess.run([WITH_SECRETS] + cmd, env=env, capture_output=True, text=True)

    def test_env_file_values_reach_the_child_only(self):
        f = self.write("s.env", "E_SECRET=five\n")
        env = dict(CLEAN, GOAT_SECRET_STORE="env-file", GOAT_SECRET_ENV_FILE=f, GOAT_BUILDER_CONFIG=self.cfg)
        r = self.run_with(env, ["sh", "-c", 'echo "$E_SECRET $GOAT_SECRETS_RESOLVED"'])
        self.assertEqual(r.stdout.strip(), "five 1", r.stderr)

    def test_env_store_runs_unchanged(self):
        env = dict(CLEAN, GOAT_SECRET_STORE="env", F_SECRET="six", GOAT_BUILDER_CONFIG=self.cfg)
        self.assertEqual(self.run_with(env, ["sh", "-c", 'echo "$F_SECRET"']).stdout.strip(), "six")

    def test_1password_calls_op_run(self):
        bindir = os.path.join(self.tmp, "bin")
        os.makedirs(bindir)
        with open(os.path.join(bindir, "op"), "w") as fh:
            fh.write('#!/bin/sh\n[ "$1" = run ] && [ "$2" = -- ] && shift 2 && G_SECRET=resolved exec "$@"\nexit 9\n')
        os.chmod(os.path.join(bindir, "op"), 0o700)
        env = dict(CLEAN, GOAT_SECRET_STORE="1password", G_SECRET="op://v/i/f", OP_ACCOUNT="x",
                   PATH=bindir + os.pathsep + os.environ["PATH"], GOAT_BUILDER_CONFIG=self.cfg)
        self.assertEqual(self.run_with(env, ["sh", "-c", 'echo "$G_SECRET"']).stdout.strip(), "resolved")

    def test_op_and_bao_references_in_one_environment_resolve_by_their_own_stores(self):
        # One grammar, scheme-dispatched: op run handles op://, and bao:// needs the
        # OpenBao injector. Without it on this checkout the run stops and says so.
        bindir = os.path.join(self.tmp, "bin")
        os.makedirs(bindir)
        with open(os.path.join(bindir, "op"), "w") as fh:
            fh.write('#!/bin/sh\nshift 2\nexec "$@"\n')
        os.chmod(os.path.join(bindir, "op"), 0o700)
        env = dict(CLEAN, GOAT_SECRET_STORE="1password", A="op://v/i/f", B="bao://goat/p/f", OP_ACCOUNT="x",
                   PATH=bindir + os.pathsep + os.environ["PATH"], GOAT_BUILDER_CONFIG=self.cfg)
        r = self.run_with(env, ["true"])
        if os.path.exists(os.path.join(REPO, "ops/secrets/openbao/run")):
            self.skipTest("the openbao injector is present; its own tests cover the chain")
        self.assertEqual(r.returncode, 127)
        self.assertIn("ops/secrets/openbao/run", r.stderr)

    def test_a_reference_store_with_no_reference_runs_unchanged(self):
        empty = os.path.join(self.tmp, "no-refs")
        os.makedirs(empty)
        env = dict(CLEAN, GOAT_SECRET_STORE="1password", GOAT_BUILDER_CONFIG=self.cfg, PATH="/usr/bin:/bin",
                   GOAT_REFS_DIR=empty)
        self.assertEqual(self.run_with(env, ["sh", "-c", "echo plain"]).stdout.strip(), "plain")

    def _refs_and_op(self):
        refs = os.path.join(self.tmp, "refs")
        os.makedirs(refs)
        with open(os.path.join(refs, "deploy.refs"), "w") as fh:
            fh.write("# references only\n"
                     "SITE_DB_PASS=op://vault/site-db/password\n"
                     "export SITE_TOKEN='op://vault/site-token/credential'\n"
                     "NOT_A_REF=plain-value\n")
        bindir = os.path.join(self.tmp, "bin")
        os.makedirs(bindir)
        # A stand-in for `op run`: every op:// value becomes "resolved:<ref>".
        with open(os.path.join(bindir, "op"), "w") as fh:
            fh.write('#!/usr/bin/env python3\n'
                     'import os, sys\n'
                     'a = sys.argv[1:]\n'
                     'assert a[0] == "run" and "--" in a, a\n'
                     'cmd = a[a.index("--") + 1:]\n'
                     'env = {k: ("resolved:" + v if v.startswith("op://") else v) for k, v in os.environ.items()}\n'
                     'os.execvpe(cmd[0], cmd, env)\n')
        os.chmod(os.path.join(bindir, "op"), 0o700)
        return refs, bindir

    def test_reference_files_resolve_with_no_shell_setup(self):
        # A fresh assembled tree links the operator's .envrc.local.d; a deploy from it
        # resolves those references with nothing exported in the shell.
        refs, bindir = self._refs_and_op()
        env = dict(CLEAN, GOAT_SECRET_STORE="1password", OP_ACCOUNT="x", GOAT_REFS_DIR=refs,
                   PATH=bindir + os.pathsep + os.environ["PATH"], GOAT_BUILDER_CONFIG=self.cfg)
        r = self.run_with(env, ["sh", "-c", 'echo "$SITE_DB_PASS|$SITE_TOKEN|${NOT_A_REF:-unset}"'])
        self.assertEqual(r.stdout.strip(),
                         "resolved:op://vault/site-db/password|resolved:op://vault/site-token/credential|unset", r.stderr)

    def test_a_variable_already_set_wins_over_the_reference_file(self):
        refs, bindir = self._refs_and_op()
        env = dict(CLEAN, GOAT_SECRET_STORE="1password", OP_ACCOUNT="x", GOAT_REFS_DIR=refs,
                   SITE_DB_PASS="op://vault/other/password",
                   PATH=bindir + os.pathsep + os.environ["PATH"], GOAT_BUILDER_CONFIG=self.cfg)
        r = self.run_with(env, ["sh", "-c", 'echo "$SITE_DB_PASS"'])
        self.assertEqual(r.stdout.strip(), "resolved:op://vault/other/password", r.stderr)

    def test_loading_reference_files_can_be_turned_off(self):
        refs, bindir = self._refs_and_op()
        env = dict(CLEAN, GOAT_SECRET_STORE="1password", OP_ACCOUNT="x", GOAT_REFS_DIR=refs,
                   GOAT_SECRETS_LOAD_REFS="0",
                   PATH=bindir + os.pathsep + os.environ["PATH"], GOAT_BUILDER_CONFIG=self.cfg)
        r = self.run_with(env, ["sh", "-c", 'echo "${SITE_DB_PASS:-unset}"'])
        self.assertEqual(r.stdout.strip(), "unset", r.stderr)

    def test_an_unknown_store_names_the_choices(self):
        env = dict(CLEAN, GOAT_SECRET_STORE="nosuch", GOAT_BUILDER_CONFIG=self.cfg)
        r = self.run_with(env, ["true"])
        self.assertEqual(r.returncode, 2)
        self.assertIn("env-file", r.stderr)


class QuickstartInit(Base):
    @unittest.skipUnless(os.path.isdir(os.path.join(REPO, "ops/ansible/group_vars/zagros_dev_hosts")),
                         "needs a declared goatnet; goatenv carries one")
    def test_generates_keeps_and_flags_external(self):
        out = os.path.join(self.tmp, "zagros_dev.env")
        with self.env():
            self.p.init("zagros_dev", out, force=False)
            self.assertEqual(stat.S_IMODE(os.stat(out).st_mode), 0o600)
            first = self.p.parse_dotenv(self.p._read(out))
            generated = {k: v for k, v in first.items() if v}
            self.assertGreater(len(generated), 10)
            self.assertTrue(all(len(v) >= 32 for v in generated.values()))
            cookies = [v for k, v in generated.items() if "COOKIE" in k]
            self.assertTrue(cookies and all(len(v) == 32 for v in cookies))
            self.assertEqual(first.get("ZAGROS_DEV_SES_SMTP_PASSWORD"), "")  # issued by an outside service
            self.p.init("zagros_dev", out, force=False)
            self.assertEqual(self.p.parse_dotenv(self.p._read(out)), first)  # a re-run never rotates

    def test_refuses_a_path_git_would_commit(self):
        path = os.path.join(REPO, "ops/secrets/not-ignored-init.env")
        try:
            with self.env():
                with self.assertRaises(self.p.PostureError):
                    self.p.init("zagros_dev", path, force=False)
            self.assertFalse(os.path.exists(path))
        finally:
            if os.path.exists(path):
                os.unlink(path)


if __name__ == "__main__":
    unittest.main()
