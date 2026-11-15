"""The bare-`?` gate (gate row C5): a `?` that passes up an uncoded error, not in the baseline, fails it."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("goatbuilder_bare_question", HERE / "goatbuilder_bare_question.py")
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


def message(file_name, text, msg="`?` couldn't convert the error to `E`"):
    """One line of cargo's JSON output: a compiler error at `file_name`, on a line reading `text`."""
    return json.dumps({
        "reason": "compiler-message",
        "message": {
            "level": "error",
            "message": msg,
            "spans": [{"is_primary": True, "file_name": file_name, "line_start": 7, "text": [{"text": "    " + text}]}],
        },
    })


class Places(unittest.TestCase):
    def test_a_bare_question_mark_is_a_place_keyed_by_file_and_line_text(self):
        got = gate.places([message("goatbuilder/src/standup.rs", "let status = child.wait()?;")], "goatbuilder/src/")
        self.assertEqual(got, [("standup.rs", "let status = child.wait()?;")])
        self.assertEqual(gate.key(got[0]), "standup.rs :: let status = child.wait()?;")

    def test_other_type_errors_of_the_copy_are_not_places(self):
        lines = [message("goatbuilder/src/standup.rs", "Err(anyhow!(\"x\"))", msg="mismatched types"),
                 json.dumps({"reason": "compiler-artifact"}), "not json"]
        self.assertEqual(gate.places(lines, "goatbuilder/src/"), [])

    def test_a_place_outside_the_crate_is_not_read(self):
        self.assertEqual(gate.places([message("goat-errors/src/lib.rs", "x?;")], "goatbuilder/src/"), [])


class Compare(unittest.TestCase):
    def test_a_new_bare_question_mark_not_in_the_baseline_fails(self):
        found = [("standup.rs", "let status = child.wait()?;"), ("tutor.rs", "let e = e?;")]
        new, gone = gate.compare(found, ["standup.rs :: let status = child.wait()?;"])
        self.assertEqual(new, ["tutor.rs :: let e = e?;"])
        self.assertEqual(gone, [])

    def test_a_listed_place_that_is_gone_fails_so_the_baseline_shrinks(self):
        new, gone = gate.compare([], ["tutor.rs :: let e = e?;"])
        self.assertEqual((new, gone), ([], ["tutor.rs :: let e = e?;"]))

    def test_the_same_line_twice_counts_twice(self):
        found = [("standup.rs", "let status = child.wait()?;")] * 3
        new, gone = gate.compare(found, ["standup.rs :: let status = child.wait()?;"] * 2)
        self.assertEqual(new, ["standup.rs :: let status = child.wait()?;"])
        self.assertEqual(gone, [])

    def test_the_listed_places_and_nothing_more_pass(self):
        found = [("a.rs", "x?;")]
        self.assertEqual(gate.compare(found, ["a.rs :: x?;"]), ([], []))


class Transform(unittest.TestCase):
    def test_the_copy_uses_the_scan_error_type_and_carries_the_canary(self):
        with tempfile.TemporaryDirectory() as d:
            src = Path(d)
            (src / "main.rs").write_text("//! the crate\nmod alpha;\nfn main() {}\n")
            (src / "alpha.rs").write_text("use anyhow::Result;\npub fn f() -> anyhow::Result<()> { Ok(()) }\n")
            gate.transform(src)
            self.assertIn("mod scanerr;\nmod alpha;", (src / "main.rs").read_text())
            alpha = (src / "alpha.rs").read_text()
            self.assertNotIn("anyhow::Result", alpha)
            self.assertIn("use crate::scanerr::Result;", alpha)
            self.assertIn(gate.CANARY, (src / "scanerr.rs").read_text())


class Baseline(unittest.TestCase):
    def test_comment_and_blank_lines_are_not_places(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "b.txt"
            p.write_text("# a comment\n\nstandup.rs :: let status = child.wait()?;\n")
            self.assertEqual(gate.read_baseline(p), ["standup.rs :: let status = child.wait()?;"])


if __name__ == "__main__":
    unittest.main()
