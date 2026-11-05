#!/usr/bin/env python3
"""The phase-13 backup row follows the site's backup declaration.

`pgbackrest-stanza-and-first-backup` requires the token `pgbackrest`, and the
acceptance runner marks a row N/A only when every service owning its tokens is
declared absent in the site's end state. No catalog service owned `pgbackrest`,
so the row applied to every site, including one that sets
has_durable_independent_state: false and defers backups to its A->B standby
(site-bootstrap.yml then never applies the pgbackrest role)."""
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


sys.path.insert(0, str(ROOT / "ops/validation"))
ENDSTATE = load("endstate", ROOT / "ops/goatbuilder/endstate/endstate.py")
RUNNER = load("run_acceptance", ROOT / "ops/validation/run-acceptance.py")


def absent_tokens(vars_text):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        gv = root / "ops/ansible/group_vars/bk_hosts"
        gv.mkdir(parents=True)
        (gv / "vars.yml").write_text("site_id: bk\n" + vars_text)
        m = ENDSTATE.model("bk", root)
        out = root / "endstate.json"
        out.write_text(json.dumps(m))
        return m, RUNNER.declared_absent(str(out))


class BackupRow(unittest.TestCase):
    def test_no_independent_state_makes_the_backup_row_not_applicable(self):
        m, gone = absent_tokens("has_durable_independent_state: false\nintra_site_pair: true\n")
        self.assertIs(m["services"]["pgbackrest"]["present"], False)
        self.assertIn("pgbackrest", gone)

    def test_a_site_with_backups_keeps_the_row(self):
        for text in ("has_durable_independent_state: true\n", ""):  # the play's default is true
            m, gone = absent_tokens(text)
            self.assertIs(m["services"]["pgbackrest"]["present"], True, text)
            self.assertNotIn("pgbackrest", gone)

    def test_the_row_still_names_the_token(self):
        import yaml
        rows = yaml.safe_load((ROOT / "ops/validation/acceptance-tests.yaml").read_text())
        rows = rows.get("tests", rows) if isinstance(rows, dict) else rows
        row = next(r for r in rows if r.get("id") == "pgbackrest-stanza-and-first-backup")
        self.assertEqual(row["requires"], ["pgbackrest"])


if __name__ == "__main__":
    unittest.main()
