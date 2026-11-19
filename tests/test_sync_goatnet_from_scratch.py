#!/usr/bin/env python3
"""ops/tutorials/sync-goatnet-from-scratch.sh against a throwaway goatcore tree and a
fake clone of hello-galaxy-goatenv.

- It replaces only the files goatcore owns. A file only the clone has, such as its
  checkpoints/base-images.txt or a note in a step directory, is kept and named,
  never deleted.
- Checkpoints are never copied: the clone rehearses tutorial 3 from a copied
  fixture, so its checkpoint differs from goatcore's in the fixture paths.
  With --goatbuilder <binary>, the clone's own ./run checks them and, under
  --apply, re-records them (./run <binary> --record, GOAT_REPO set to this
  checkout). Without it, the checkpoints are not compared, and the exit code
  says so (3)."""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
SCRIPT = "ops/tutorials/sync-goatnet-from-scratch.sh"
STEPS = ("00-goatlite", "01-founder", "02-first-federated-node", "03-boundary-node")

# The clone's ./run: records its arguments and GOAT_REPO; with --record it writes
# one checkpoint; without, it exits as RUN_RC says (the rehearsal's verdict).
RUN = """#!/bin/sh
here="$(cd "$(dirname "$0")" && pwd)"
printf '%s|%s\\n' "$*" "${GOAT_REPO:-}" >> "$here/run.log"
case " $* " in *" --record "*) echo "recorded by run" > "$here/checkpoints/03-boundary-node.txt"; exit 0;; esac
exit "${RUN_RC:-0}"
"""


def goatcore() -> Path:
    root = Path(tempfile.mkdtemp())
    (root / SCRIPT).parent.mkdir(parents=True)
    shutil.copy(REPO / SCRIPT, root / SCRIPT)
    fx = root / "ops/goatbuilder/fixtures/goatnet-from-scratch"
    for s in STEPS:
        (fx / s).mkdir(parents=True)
        (fx / s / "goatbuilder.yaml").write_text(f"step: {s}\n")
    (fx / "02-first-federated-node/records").mkdir()
    (fx / "02-first-federated-node/records/a.jsonl").write_text("{}\n")
    (fx / "checkpoints").mkdir()
    (fx / "checkpoints/03-boundary-node.txt").write_text("goatcore's own paths\n")
    env = {"PATH": os.environ["PATH"], "HOME": str(root), "GIT_CONFIG_GLOBAL": "/dev/null"}
    for cmd in (["git", "init", "-q"], ["git", "add", "-A"],
                ["git", "-c", "user.name=t", "-c", "user.email=t@example.org", "-c", "commit.gpgsign=false",
                 "commit", "-q", "-m", "fixture"]):
        subprocess.run(cmd, cwd=root, check=True, env=env)
    return root


def clone(core: Path) -> Path:
    dest = Path(tempfile.mkdtemp())
    fx = core / "ops/goatbuilder/fixtures/goatnet-from-scratch"
    for s in STEPS:
        shutil.copytree(fx / s, dest / s)
    (dest / "checkpoints").mkdir()
    (dest / "checkpoints/03-boundary-node.txt").write_text("the clone's fixture paths\n")
    (dest / "checkpoints/base-images.txt").write_text("only the clone has this\n")
    (dest / "01-founder/NOTES.md").write_text("only the clone has this too\n")
    (dest / "run").write_text(RUN)
    (dest / "run").chmod(0o755)
    (dest / "GOATBUILDER_REF").write_text("old\n")
    return dest


def sync(core: Path, *args, rc_of_run: int = 0):
    env = {"PATH": os.environ["PATH"], "HOME": str(core), "RUN_RC": str(rc_of_run)}
    return subprocess.run(["bash", str(core / SCRIPT), *map(str, args)], capture_output=True, text=True, env=env, timeout=60)


class Sync(unittest.TestCase):
    def setUp(self):
        self.core = goatcore()
        self.dest = clone(self.core)
        self.bin = self.core / "goatbuilder-stand-in"
        self.bin.write_text("#!/bin/sh\n")
        self.bin.chmod(0o755)

    def tearDown(self):
        shutil.rmtree(self.core)
        shutil.rmtree(self.dest)

    def kept(self):
        return ((self.dest / "checkpoints/base-images.txt").read_text(),
                (self.dest / "01-founder/NOTES.md").read_text())

    def test_equal_steps_without_a_binary_say_the_checkpoints_were_not_compared(self):
        r = sync(self.core, self.dest)
        self.assertEqual(r.returncode, 3, r.stdout + r.stderr)
        self.assertIn("checkpoints", r.stdout)
        self.assertIn("not compared", r.stdout)
        self.assertIn("01-founder/NOTES.md", r.stdout, "a clone-only file is named")
        self.assertNotIn("DIFF", r.stdout)

    def test_a_step_that_differs_fails_the_comparison(self):
        (self.dest / "02-first-federated-node/goatbuilder.yaml").write_text("edited\n")
        r = sync(self.core, self.dest, "--goatbuilder", self.bin)
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("DIFF  02-first-federated-node/goatbuilder.yaml", r.stdout)

    def test_apply_replaces_goatcores_files_and_keeps_the_clones_own(self):
        fx = self.core / "ops/goatbuilder/fixtures/goatnet-from-scratch"
        (fx / "02-first-federated-node/goatbuilder.yaml").write_text("changed in goatcore\n")
        (fx / "03-boundary-node/new.yaml").write_text("new in goatcore\n")
        r = sync(self.core, self.dest, "--apply")
        self.assertEqual(r.returncode, 3, r.stdout + r.stderr)
        self.assertEqual((self.dest / "02-first-federated-node/goatbuilder.yaml").read_text(), "changed in goatcore\n")
        self.assertEqual((self.dest / "03-boundary-node/new.yaml").read_text(), "new in goatcore\n")
        self.assertEqual(self.kept(), ("only the clone has this\n", "only the clone has this too\n"))
        self.assertEqual((self.dest / "checkpoints/03-boundary-node.txt").read_text(), "the clone's fixture paths\n",
                         "no checkpoint is copied from goatcore")
        head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=self.core, capture_output=True, text=True).stdout.strip()
        self.assertEqual((self.dest / "GOATBUILDER_REF").read_text().strip(), head)
        self.assertIn("not re-recorded", r.stdout)

    def test_apply_with_a_binary_re_records_the_checkpoints_through_the_clones_run(self):
        r = sync(self.core, self.dest, "--apply", "--goatbuilder", self.bin)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual((self.dest / "checkpoints/03-boundary-node.txt").read_text(), "recorded by run\n")
        self.assertEqual(self.kept(), ("only the clone has this\n", "only the clone has this too\n"))
        log = (self.dest / "run.log").read_text().splitlines()
        self.assertEqual(log, [f"{self.bin} --record|{self.core}"], "./run <binary> --record, GOAT_REPO = this checkout")

    def test_compare_with_a_binary_takes_the_clones_rehearsal_verdict(self):
        self.assertEqual(sync(self.core, self.dest, "--goatbuilder", self.bin).returncode, 0)
        self.assertEqual((self.dest / "run.log").read_text().splitlines(), [f"{self.bin}|{self.core}"])
        r = sync(self.core, self.dest, "--goatbuilder", self.bin, rc_of_run=1)
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("DIFF  checkpoints", r.stdout)

    def test_a_tree_without_run_is_refused(self):
        (self.dest / "run").unlink()
        self.assertEqual(sync(self.core, self.dest).returncode, 2)


if __name__ == "__main__":
    unittest.main()
