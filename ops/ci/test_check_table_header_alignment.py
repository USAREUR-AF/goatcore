"""Regression tests for the table header alignment guard."""

import importlib.util
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "check_table_header_alignment", HERE / "check-table-header-alignment.py")
guard = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(guard)


class TableHeaderAlignmentTest(unittest.TestCase):
    def setUp(self):
        temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(temporary_directory.cleanup)
        self.root = Path(temporary_directory.name)
        self.path = self.root / "ops" / "example" / "www" / "table.html"
        self.path.parent.mkdir(parents=True)

    def scan(self, text):
        self.path.write_text(text, encoding="utf-8")
        return guard.scan(self.root)

    def test_misaligned_numeric_header_yields_finding(self):
        findings = self.scan(
            '<style>td.n{text-align:right}</style><table><th class="n">Count</th></table>')
        self.assertEqual(findings, [(Path("ops/example/www/table.html"), "n")])

    def test_combined_data_and_header_rule_yields_no_finding(self):
        findings = self.scan(
            '<style>td.n,th.n{text-align:right}</style><table><th class="n">Count</th></table>')
        self.assertEqual(findings, [])

    def test_commented_header_rule_does_not_hide_finding(self):
        comments = (
            "/* th.n{text-align:right} */",
            "<!-- <style>th.n{text-align:right}</style> -->",
        )
        for comment in comments:
            with self.subTest(comment=comment):
                findings = self.scan(
                    f'<style>td.n{{text-align:right}}</style>{comment}'
                    '<table><th class="n">Count</th></table>')
                self.assertEqual(findings, [(Path("ops/example/www/table.html"), "n")])

    def test_data_only_class_yields_no_finding(self):
        findings = self.scan(
            '<style>td.n{text-align:right}</style><table><td class="n">1</td></table>')
        self.assertEqual(findings, [])


if __name__ == "__main__":
    unittest.main()
