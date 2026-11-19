#!/usr/bin/env python3
"""ops/scripts/join-site-secrets.py: a joining site takes its founder's shared secrets and
mints the rest (docs/design/join-node-site-secrets.md). No live store, account or host."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SHARED = ["HYDRA_SYSTEM", "KRATOS_CIPHER", "NB_ENCKEY", "ORY_SHARED_NID", "RELAY_SECRET", "REPLICATOR_PASS"]
FOUNDER = {x: f"founder-value-{x.lower()}-0123456789abcdef" for x in SHARED}
SCRIPTS = ("newsite-secrets.sh", "newsite-secret-store.py", "join-site-secrets.py", "with-secrets")


class Checkout:
    """A git checkout with the scripts, the posture and a reference directory."""

    def __init__(self, tmp):
        self.root = Path(tmp)
        (self.root / "ops/scripts").mkdir(parents=True)
        (self.root / "ops/secrets").mkdir(parents=True)
        for name in SCRIPTS:
            shutil.copy(ROOT / "ops/scripts" / name, self.root / "ops/scripts" / name)
        shutil.copy(ROOT / "ops/secrets/posture.py", self.root / "ops/secrets/posture.py")
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        (self.root / ".gitignore").write_text(".envrc.local.d/\n")
        self.refs = self.root / ".envrc.local.d"
        self.refs.mkdir(mode=0o700)
        self.env = {"PATH": os.environ["PATH"], "HOME": str(self.root), "GOAT_SECRETS_SESSION": "0"}

    def run(self, *args, extra=None):
        env = dict(self.env, **(extra or {}))
        return subprocess.run(["python3", str(self.root / "ops/scripts/join-site-secrets.py"), *args],
                              env=env, capture_output=True, text=True, cwd=self.root)


class CustomStore(unittest.TestCase):
    """A store of references: an adapter that writes a JSON vault and a runner that resolves
    test:// references from it, as GOAT_SECRET_INIT and GOAT_SECRET_RUN do."""

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.c = Checkout(tmp.name)
        self.vault = self.c.root / "vault.json"
        self.vault.write_text(json.dumps({f"test://zagros-a/ZAGROS_A_{x}": v for x, v in FOUNDER.items()}))
        adapter, runner = self.c.root / "adapter", self.c.root / "runner"
        adapter.write_text(f'''#!/usr/bin/env python3
import json,sys
p=json.load(sys.stdin); v=json.load(open({str(self.vault)!r}))
out={{}}
for n,val in p["secrets"].items():
    ref="test://"+p["site"]+"/"+n; v.setdefault(ref,val); out[n]=ref
json.dump(v,open({str(self.vault)!r},"w")); json.dump(out,sys.stdout)
''')
        runner.write_text(f'''#!/usr/bin/env python3
import json,os,sys
v=json.load(open({str(self.vault)!r}))
env=dict(os.environ)
for k,val in list(env.items()):
    if val.startswith("test://") and val in v: env[k]=v[val]
os.execvpe(sys.argv[1], sys.argv[1:], env)
''')
        adapter.chmod(0o700)
        runner.chmod(0o700)
        self.c.env.update({"GOAT_SECRET_STORE": "custom", "GOAT_SECRET_INIT": str(adapter),
                           "GOAT_SECRET_RUN": str(runner), "GOAT_SECRET_REF_PREFIX": "test://"})
        self.target = self.c.refs / "30-site-services.refs"
        self.target.write_text("".join(f'export ZAGROS_A_{x}="test://zagros-a/ZAGROS_A_{x}"\n' for x in SHARED))

    def refs(self):
        out = {}
        for line in self.target.read_text().splitlines():
            if line.startswith("export "):
                k, _, v = line[len("export "):].partition("=")
                out[k] = v.strip('"')
        return out

    def test_a_joiner_takes_the_founders_references_and_mints_the_rest(self):
        r = self.c.run("zagros-b", "--founder", "zagros-a")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        refs = self.refs()
        for x in SHARED:
            self.assertEqual(refs[f"ZAGROS_B_{x}"], f"test://zagros-a/ZAGROS_A_{x}", x)
            self.assertIn(f"ZAGROS_B_{x}: same as ZAGROS_A_{x}", r.stdout)
        # The site's own values are minted, under the joiner's own entries.
        self.assertEqual(refs["ZAGROS_B_PG_PASS"], "test://zagros-b/ZAGROS_B_PG_PASS")
        self.assertEqual(refs["ZAGROS_B_BKP_KEY"], "test://zagros-b/ZAGROS_B_BKP_KEY")
        vault = json.loads(self.vault.read_text())
        self.assertFalse(any(k.startswith(f"test://zagros-b/ZAGROS_B_{x}") for k in vault for x in SHARED),
                         "no shared value is stored a second time")
        for v in FOUNDER.values():
            self.assertNotIn(v, r.stdout + r.stderr)
        self.assertEqual(oct(self.target.stat().st_mode & 0o777), "0o600")

    def test_a_second_run_changes_nothing(self):
        self.assertEqual(self.c.run("zagros-b", "--founder", "zagros-a").returncode, 0)
        before = self.target.read_text()
        r = self.c.run("zagros-b", "--founder", "zagros-a")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.target.read_text(), before)
        self.assertIn("ZAGROS_B_NB_ENCKEY: already names ZAGROS_A_NB_ENCKEY's reference", r.stdout)

    def test_a_founder_without_a_reference_is_refused_and_nothing_is_written(self):
        self.target.write_text("".join(f'export ZAGROS_A_{x}="test://zagros-a/ZAGROS_A_{x}"\n'
                                       for x in SHARED if x != "NB_ENCKEY"))
        before = self.target.read_text()
        r = self.c.run("zagros-b", "--founder", "zagros-a")
        self.assertEqual(r.returncode, 1)
        self.assertIn("GB-352: ZAGROS_A_NB_ENCKEY has no test:// reference", r.stderr)
        self.assertEqual(self.target.read_text(), before)

    def test_a_joiner_holding_a_different_value_is_refused_and_kept(self):
        vault = json.loads(self.vault.read_text())
        vault["test://zagros-b/ZAGROS_B_NB_ENCKEY"] = "a-fresh-value-minted-by-hand-0000000000"
        self.vault.write_text(json.dumps(vault))
        with self.target.open("a") as fh:
            fh.write('export ZAGROS_B_NB_ENCKEY="test://zagros-b/ZAGROS_B_NB_ENCKEY"\n')
        r = self.c.run("zagros-b", "--founder", "zagros-a")
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("ZAGROS_B_NB_ENCKEY: differs from ZAGROS_A_NB_ENCKEY", r.stdout)
        self.assertIn("GB-353: ZAGROS_B_NB_ENCKEY already hold", r.stderr)
        self.assertEqual(self.refs()["ZAGROS_B_NB_ENCKEY"], "test://zagros-b/ZAGROS_B_NB_ENCKEY", "never overwritten")
        self.assertNotIn("a-fresh-value", r.stdout + r.stderr)

    def test_a_dry_run_writes_nothing(self):
        before = self.target.read_text()
        r = self.c.run("zagros-b", "--founder", "zagros-a", "--dry-run")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("ZAGROS_B_NB_ENCKEY takes ZAGROS_A_NB_ENCKEY (the founder's reference)", r.stdout)
        self.assertIn("would generate: ZAGROS_B_PG_PASS", r.stdout)
        self.assertNotIn("would generate: ZAGROS_B_NB_ENCKEY", r.stdout)
        self.assertEqual(self.target.read_text(), before)

    def test_a_founder_takes_nothing_from_itself(self):
        r = self.c.run("zagros-a", "--founder", "zagros-a")
        self.assertEqual(r.returncode, 1)
        self.assertIn("a founder takes nothing from itself", r.stderr)


class EnvFileStore(unittest.TestCase):
    """A store of values: the posture's gitignored values file."""

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.c = Checkout(tmp.name)
        self.file = self.c.refs / "secrets.env"
        self.file.write_text("".join(f"ZAGROS_A_{x}={v}\n" for x, v in FOUNDER.items()))
        self.file.chmod(0o600)
        self.c.env.update({"GOAT_SECRET_STORE": "env-file", "GOAT_SECRET_ENV_FILE": str(self.file)})

    def values(self):
        return dict(line.split("=", 1) for line in self.file.read_text().splitlines()
                    if "=" in line and not line.startswith("#"))

    def test_a_joiner_takes_the_founders_values_and_mints_the_rest(self):
        r = self.c.run("zagros-b", "--founder", "zagros-a")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        v = self.values()
        for x in SHARED:
            self.assertEqual(v[f"ZAGROS_B_{x}"], FOUNDER[x], x)
            self.assertIn(f"ZAGROS_B_{x}: same as ZAGROS_A_{x}", r.stdout)
        self.assertTrue(v["ZAGROS_B_PG_PASS"])
        self.assertNotEqual(v["ZAGROS_B_PG_PASS"], v.get("ZAGROS_A_PG_PASS"))
        for val in FOUNDER.values():
            self.assertNotIn(val, r.stdout + r.stderr)

    def test_a_joiner_holding_a_different_value_is_refused_and_kept(self):
        with self.file.open("a") as fh:
            fh.write("ZAGROS_B_REPLICATOR_PASS=minted-by-hand-0000000000000000\n")
        r = self.c.run("zagros-b", "--founder", "zagros-a")
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("GB-353: ZAGROS_B_REPLICATOR_PASS already holds", r.stderr)
        self.assertEqual(self.values()["ZAGROS_B_REPLICATOR_PASS"], "minted-by-hand-0000000000000000")
        self.assertNotIn("ZAGROS_B_NB_ENCKEY", self.values(), "nothing written after a refusal")

    def test_a_founder_without_a_value_is_refused(self):
        self.file.write_text("".join(f"ZAGROS_A_{x}={v}\n" for x, v in FOUNDER.items() if x != "HYDRA_SYSTEM"))
        r = self.c.run("zagros-b", "--founder", "zagros-a")
        self.assertEqual(r.returncode, 1)
        self.assertIn("GB-352: ZAGROS_A_HYDRA_SYSTEM has no value", r.stderr)
        self.assertNotIn("ZAGROS_B_NB_ENCKEY", self.values())


class EnvStore(unittest.TestCase):
    def test_unexported_joiner_names_are_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            c = Checkout(tmp)
            env = {"GOAT_SECRET_STORE": "env", **{f"ZAGROS_A_{x}": v for x, v in FOUNDER.items()}}
            r = c.run("zagros-b", "--founder", "zagros-a", extra=env)
            self.assertEqual(r.returncode, 1)
            self.assertIn("GB-354: the env store holds nothing", r.stderr)


class TheSharedClass(unittest.TestCase):
    def test_the_spec_marks_the_six_names_the_credentialing_page_lists(self):
        out = subprocess.run(["bash", str(ROOT / "ops/scripts/newsite-secrets.sh"), "zagros-b", "--cluster-json"],
                             capture_output=True, text=True, env={"PATH": os.environ["PATH"], "GOAT_SECRET_STORE": "custom"})
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(json.loads(out.stdout), [f"ZAGROS_B_{x}" for x in SHARED])
        page = (ROOT / "docs/operations/host-site-node-credentialing.md").read_text()
        row = next(line for line in page.splitlines() if line.startswith("| **Cluster-shared"))
        self.assertEqual(sorted(x for x in SHARED if f"ZAGROS_{x}" in row), SHARED)

    def test_spec_json_keeps_its_shape(self):
        out = subprocess.run(["bash", str(ROOT / "ops/scripts/newsite-secrets.sh"), "zagros-b", "--spec-json"],
                             capture_output=True, text=True, env={"PATH": os.environ["PATH"], "GOAT_SECRET_STORE": "custom"})
        spec = json.loads(out.stdout)
        self.assertEqual(spec["ZAGROS_B_NB_ENCKEY"], "b64_32")
        self.assertEqual(spec["ZAGROS_B_HYDRA_SYSTEM"], "hex32")


if __name__ == "__main__":
    unittest.main()
