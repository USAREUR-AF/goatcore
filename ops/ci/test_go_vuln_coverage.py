"""Red-first fixtures for Go module scan coverage; no cloud access."""
import copy
import unittest

import check_go_vuln_coverage as guard


def scan(directory):
    return {"working-directory": directory, "run": "govulncheck ./...\n"}


class CoverageTests(unittest.TestCase):
    def setUp(self):
        # One exemption of the fixture's own, whatever the repository declares.
        self._exemptions = guard.EXEMPTIONS
        guard.EXEMPTIONS = {"ops/example-tool": "fixture tooling"}
        self.addCleanup(setattr, guard, "EXEMPTIONS", self._exemptions)
        self.ci = {"jobs": {"go-vuln": {
            "strategy": {"matrix": {"module": ["ops/example"]}},
            "steps": [scan("${{ matrix.module }}")],
        }}}
        self.own = {"jobs": {"test": {"steps": [scan("signal-server")]}}}
        self.modules = {"ops/example", "signal-server", *guard.EXEMPTIONS}

    def check(self, modules=None, ci=None, own=None):
        return guard.validate(modules or self.modules, ci or self.ci,
                              own or self.own)

    def test_complete_partition_passes(self):
        self.assertEqual(self.check(), [])

    def test_new_module_without_scan_fails(self):
        self.assertTrue(self.check(self.modules | {"ops/new-service"}))

    def test_stale_exemption_fails(self):
        self.assertTrue(self.check(self.modules - {next(iter(guard.EXEMPTIONS))}))

    def test_duplicate_and_stale_matrix_rows_fail(self):
        for row in ("ops/example", "ops/missing", "signal-server"):
            ci = copy.deepcopy(self.ci)
            ci["jobs"]["go-vuln"]["strategy"]["matrix"]["module"].append(row)
            self.assertTrue(self.check(ci=ci), row)

    def test_comment_or_echo_does_not_count_as_scan(self):
        for command in ("# govulncheck ./...", "echo govulncheck ./...", "true",
                        "if false; then\ngovulncheck ./...\nfi",
                        "cat <<EOF\ngovulncheck ./...\nEOF",
                        "govulncheck ./...\ntrue"):
            own = copy.deepcopy(self.own)
            own["jobs"]["test"]["steps"][0]["run"] = command
            self.assertTrue(self.check(own=own), command)

    def test_disabled_or_nonblocking_scans_fail(self):
        for option in ("if", "continue-on-error"):
            for value in (True, "false"):
                own = copy.deepcopy(self.own)
                own["jobs"]["test"]["steps"][0][option] = value
                # Any conditional scan is refused. Explicit false for
                # continue-on-error remains a blocking step.
                errors = self.check(own=own)
                self.assertEqual(bool(errors), option == "if" or value is True)

    def test_scan_at_wrong_directory_fails(self):
        own = copy.deepcopy(self.own)
        own["jobs"]["test"]["steps"][0]["working-directory"] = "ops/example"
        self.assertTrue(self.check(own=own))

    def test_matrix_without_real_scan_fails(self):
        self.ci["jobs"]["go-vuln"]["steps"] = []
        self.assertTrue(self.check())

    def test_matrix_exclusions_cannot_bypass_coverage(self):
        matrix = self.ci["jobs"]["go-vuln"]["strategy"]["matrix"]
        matrix["exclude"] = []
        self.assertEqual(self.check(), [])
        matrix["exclude"] = [{"module": "ops/example"}]
        self.assertTrue(self.check())


if __name__ == "__main__":
    unittest.main()
