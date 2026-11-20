#!/usr/bin/env python3
"""Tutorial 00 says how a goatlite with no route to the trunk stands up.

Before it touches a host, a real goatbuilder run fetches the trunk to prove the
checkout is current (contract C3), and refuses with GB-065 when the fetch fails.
A goatlite on a network with no route to the forge therefore needs --offline,
and the tutorial must say so where a learner who hits GB-065 looks: its
"When something fails" section. That section is after "## 8. Use it", so the
first run's budget (crates/goatbuilder/tests/goatlite_budget.rs) is unchanged.
GB-065's own fix must name --offline too, since the error is what the learner
sees first.
"""
import re
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
PAGE = ROOT / "docs/tutorials/00-goatlite.md"


def section(text, heading):
    """The body under `## <heading>`, up to the next `## ` heading."""
    m = re.search(rf"^## {re.escape(heading)}\n(.*?)(?=^## |\Z)", text, re.S | re.M)
    return m.group(1) if m else ""


class TutorialNamesOffline(unittest.TestCase):
    def test_when_something_fails_names_gb_065_and_offline(self):
        text = PAGE.read_text()
        body = section(text, "When something fails")
        self.assertTrue(body, "no '## When something fails' section in tutorial 00")
        self.assertIn("GB-065", body, "the section does not name GB-065, the refusal a run with no route to the trunk meets")
        self.assertIn("--offline", body, "the section does not say to pass --offline")
        self.assertRegex(body, r"goatbuilder standup \S+ [^`\n]*--offline",
                         "the section gives no standup command with --offline")
        # It says what --offline skips and what it does not.
        self.assertRegex(body, r"(?i)fetch", "the section does not say --offline skips the trunk fetch")
        self.assertRegex(body, r"(?i)image", "the section does not say the images still have to reach the box")
        # Kept out of the first run's budget: after "## 8. Use it".
        self.assertLess(text.index("## 8. Use it"), text.index("## When something fails"))

    def test_gb_065_names_offline_as_the_way_through(self):
        catalog = yaml.safe_load((ROOT / "ops/errors/catalog.yaml").read_text())
        rows = [e for e in catalog["entries"] if e.get("code") == "GB-065"]
        self.assertEqual(len(rows), 1, "GB-065 is not in ops/errors/catalog.yaml once")
        self.assertTrue(any("--offline" in f for f in rows[0]["fixes"]), "GB-065's fixes do not name --offline")


if __name__ == "__main__":
    unittest.main()
