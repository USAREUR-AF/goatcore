#!/usr/bin/env python3
"""ops/scripts/goat-standup.sh reads <repo>/goatlite.yaml the way `goatbuilder standup`
does (standup.rs resolve_lite): a file it cannot read as YAML refuses the run, and a
file that names this site and does not validate refuses it too. Neither is taken as
"not a goatlite", which planned a different standup from the one goatbuilder plans."""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]


def checkout(goatlite: str) -> Path:
    """A throwaway tree holding the script, the validator and the schema it reads."""
    root = Path(tempfile.mkdtemp())
    for rel in ("ops/scripts/goat-standup.sh", "ops/goatbuilder/validate.py"):
        (root / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(REPO / rel, root / rel)
    shutil.copytree(REPO / "ops/goatbuilder/schema", root / "ops/goatbuilder/schema")
    (root / "goatlite.yaml").write_text(goatlite)
    return root


def plan(root: Path):
    env = {"PATH": os.environ["PATH"], "HOME": tempfile.mkdtemp()}
    return subprocess.run(["bash", str(root / "ops/scripts/goat-standup.sh"), "pasture", "--plan"],
                          capture_output=True, text=True, env=env, timeout=120)


FOUR = "schema: goatbuilder/v1\npreset: lite\nlite:\n  name: pasture\n  host: 192.0.2.50\n  ssh_user: ubuntu\n  admin: me@example.org\n"


class GoatliteFile(unittest.TestCase):
    def test_a_file_that_is_not_yaml_refuses(self):
        r = plan(checkout("schema: goatbuilder/v1\npreset: lite\nlite: [unclosed\n"))
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("goatlite.yaml", r.stderr)

    def test_a_file_naming_this_site_that_does_not_validate_refuses(self):
        r = plan(checkout(FOUR.replace("  admin: me@example.org\n", "")))
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("goatlite.yaml", r.stderr)

    def test_a_file_naming_another_goatnet_is_left_alone(self):
        r = plan(checkout(FOUR.replace("name: pasture", "name: meadow").replace("  admin: me@example.org\n", "")))
        self.assertNotIn("goatlite.yaml does not", r.stderr)


if __name__ == "__main__":
    unittest.main()
