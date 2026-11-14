#!/usr/bin/env python3
"""ops/scripts/project-setting.sh reads a declared value the way goatbuilder's YAML
reader does, or refuses it by name. A double-quoted scalar with escapes, or a
single-quoted one with a doubled quote, is unescaped through PyYAML; without PyYAML
the script refuses it (exit 3, naming the key) rather than printing the escapes,
which would leave the shell drivers with a different value from goatbuilder's."""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]


def tree(state_dir_line: str) -> Path:
    root = Path(tempfile.mkdtemp())
    (root / "ops/scripts").mkdir(parents=True)
    (root / "ops/goatbuilder").mkdir(parents=True)
    shutil.copy(REPO / "ops/scripts/project-setting.sh", root / "ops/scripts/project-setting.sh")
    (root / "ops/goatbuilder/goatbuilder.yaml").write_text(f"schema: goatbuilder/v1\ndeploy:\n  state_dir: {state_dir_line}\n")
    return root


def read(root: Path, path: str, *args: str):
    return subprocess.run(["bash", str(root / "ops/scripts/project-setting.sh"), *args],
                          capture_output=True, text=True, env={"PATH": path, "HOME": tempfile.mkdtemp()})


def no_pyyaml_path() -> str:
    """PATH whose python3 cannot import yaml (a shim that fails), with bash and awk."""
    shim = Path(tempfile.mkdtemp())
    (shim / "python3").write_text("#!/bin/sh\nexit 1\n")
    (shim / "python3").chmod(0o755)
    return f"{shim}:{os.environ['PATH']}"


class Quoted(unittest.TestCase):
    def test_plain_and_simply_quoted_values_read_as_before(self):
        for line, want in [("~/.goat/state", "~/.goat/state"), ('"~/goat state"', "~/goat state"), ("'/srv/goat'", "/srv/goat")]:
            for path in (os.environ["PATH"], no_pyyaml_path()):
                r = read(tree(line), path, "deploy", "state_dir")
                self.assertEqual((r.returncode, r.stdout.strip()), (0, want), r.stderr)

    def test_escapes_are_unescaped_as_yaml_reads_them(self):
        try:
            import yaml  # noqa: F401
        except ImportError:
            self.skipTest("PyYAML not installed here")
        for line, want in [('"~/a\\"z\\\\b"', '~/a"z\\b'), ("'it''s'", "it's")]:
            r = read(tree(line), os.environ["PATH"], "deploy", "state_dir")
            self.assertEqual((r.returncode, r.stdout.rstrip("\n")), (0, want), r.stderr)
            r = read(tree(line), os.environ["PATH"], "ca-audit-log")
            self.assertEqual(r.returncode, 0, r.stderr)

    def test_escapes_without_pyyaml_are_refused_by_name(self):
        for line in ['"~/a\\"z\\\\b"', "'it''s'"]:
            r = read(tree(line), no_pyyaml_path(), "deploy", "state_dir")
            self.assertEqual(r.returncode, 3, r.stdout)
            self.assertIn("deploy.state_dir", r.stderr)
            self.assertEqual(r.stdout, "")


    def test_goat_standup_refuses_rather_than_reading_it_differently(self):
        root = tree('"~/a\\"z\\\\b"')
        shutil.copy(REPO / "ops/scripts/goat-standup.sh", root / "ops/scripts/goat-standup.sh")
        r = subprocess.run(["bash", str(root / "ops/scripts/goat-standup.sh"), "alpha", "--plan"],
                           capture_output=True, text=True, timeout=120,
                           env={"PATH": no_pyyaml_path(), "HOME": tempfile.mkdtemp()})
        self.assertEqual(r.returncode, 2, r.stdout[-400:])
        self.assertIn("deploy.state_dir", r.stderr)


    def test_goat_standup_refuses_an_aws_profile_it_cannot_read_as_goatbuilder_does(self):
        root = tree("~/.goat/state")
        (root / "ops/goatbuilder/goatbuilder.yaml").write_text('schema: goatbuilder/v1\ndeploy:\n  aws_profile: "p\\"q"\n')
        shutil.copy(REPO / "ops/scripts/goat-standup.sh", root / "ops/scripts/goat-standup.sh")
        r = subprocess.run(["bash", str(root / "ops/scripts/goat-standup.sh"), "alpha", "--plan"],
                           capture_output=True, text=True, timeout=120,
                           env={"PATH": no_pyyaml_path(), "HOME": tempfile.mkdtemp()})
        self.assertEqual(r.returncode, 2, r.stdout[-400:])
        self.assertIn("deploy.aws_profile", r.stderr)


    def test_goat_standup_stops_on_a_config_it_cannot_read(self):
        if os.geteuid() == 0:
            self.skipTest("root reads a mode-000 file")
        root = tree("~/.goat/state")
        shutil.copy(REPO / "ops/scripts/goat-standup.sh", root / "ops/scripts/goat-standup.sh")
        (root / "ops/goatbuilder/goatbuilder.yaml").chmod(0)
        r = subprocess.run(["bash", str(root / "ops/scripts/goat-standup.sh"), "alpha", "--plan"],
                           capture_output=True, text=True, timeout=120,
                           env={"PATH": no_pyyaml_path(), "HOME": tempfile.mkdtemp()})
        self.assertEqual(r.returncode, 2, r.stdout[-300:])
        self.assertIn("goatbuilder.yaml", r.stderr)


class JoinFounderSite(unittest.TestCase):
    """The ACL site names an environment variable (<SITE>_NB_API_TOKEN), so
    goat-standup.sh refuses a join_founder_site that cannot make one, as goatbuilder does."""

    def plan(self, jfs: str):
        root = tree("~/.goat/state")
        shutil.copy(REPO / "ops/scripts/goat-standup.sh", root / "ops/scripts/goat-standup.sh")
        gv = root / "ops/ansible/group_vars/alpha_hosts"
        gv.mkdir(parents=True)
        (gv / "vars.yml").write_text(f"goatnet_founder: false\njoin_founder_site: {jfs}\n")
        return subprocess.run(["bash", str(root / "ops/scripts/goat-standup.sh"), "alpha", "--plan"],
                              capture_output=True, text=True, timeout=120,
                              env={"PATH": os.environ["PATH"], "HOME": tempfile.mkdtemp()})

    def test_one_that_cannot_name_a_variable_is_refused(self):
        for jfs in ["a$(x)", "a;b", "a`b`"]:
            r = self.plan(jfs)
            self.assertEqual(r.returncode, 2, r.stdout[-300:])
            self.assertIn("join_founder_site", r.stderr)

    def test_a_founder_site_that_cannot_name_one_is_refused_naming_the_site(self):
        root = tree("~/.goat/state")
        shutil.copy(REPO / "ops/scripts/goat-standup.sh", root / "ops/scripts/goat-standup.sh")
        r = subprocess.run(["bash", str(root / "ops/scripts/goat-standup.sh"), "a;b", "--plan"],
                           capture_output=True, text=True, timeout=120,
                           env={"PATH": os.environ["PATH"], "HOME": tempfile.mkdtemp()})
        self.assertEqual(r.returncode, 2, r.stdout[-300:])
        self.assertIn("the site 'a;b'", r.stderr)
        self.assertNotIn("join_founder_site", r.stderr)

    def test_an_ordinary_one_plans(self):
        r = self.plan("bravo-1")
        self.assertNotIn("join_founder_site", r.stderr)
        self.assertIn("BRAVO_1_NB_API_TOKEN", r.stdout)


if __name__ == "__main__":
    unittest.main()
