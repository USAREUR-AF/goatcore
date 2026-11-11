#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Known answers for ops/ci/ci_areas.py: what each kind of change needs.

Each case is a real shape from recent pull requests. A wrong answer here
means a job would be skipped for a change it reads, or run for one it cannot
see, so every case asserts the full plan, not one job.
"""
import importlib.util
import pathlib
import unittest

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("ci_areas", HERE / "ci_areas.py")
ci_areas = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci_areas)

EVERY = set(ci_areas.JOBS)
ALWAYS = set(ci_areas.ALWAYS)


def needed(paths):
    plan, _ = ci_areas.plan(paths)
    return {job for job, need in plan.items() if need}


class Plan(unittest.TestCase):
    def test_docs_only_needs_only_the_always_on_jobs(self):
        self.assertEqual(needed(["docs/design/paging.md", "README.md"]), ALWAYS)

    def test_alerting_templates_skip_go_and_peering(self):
        # The #420 shape: two Alertmanager templates and a render test.
        got = needed([
            "ops/ansible/roles/obs-host/templates/alertmanager.yml.j2",
            "ops/ansible/roles/observability/templates/alertmanager.yml.j2",
            "tests/test_alert_channels_off_when_empty.py",
        ])
        # goatbuilder reads ops/ansible, so rust stays on; no Go, no go-vuln,
        # no host-firewall suite.
        self.assertEqual(got, ALWAYS | {"rust", "python-and-ansible"})

    def test_go_module_change_needs_go_and_go_vuln(self):
        got = needed(["ops/goat-zenoh-portal/internal/access/coalition.go"])
        self.assertEqual(got, ALWAYS | {"go", "go-vuln", "python-and-ansible"})

    def test_go_mod_alone_needs_go_vuln(self):
        self.assertIn("go-vuln", needed(["ops/snitch/go.mod"]))

    def test_rust_crate_change_skips_go(self):
        got = needed(["crates/goat-cli/src/main.rs"])
        self.assertEqual(got, ALWAYS | {"rust"})

    def test_goatbuilder_federate_source_also_needs_python(self):
        # Python tests read crates/goatbuilder/src/federate.
        got = needed(["crates/goatbuilder/src/federate/preflight.rs"])
        self.assertEqual(got, ALWAYS | {"rust", "python-and-ansible"})

    def test_host_firewall_role_needs_both_peering_suites(self):
        got = needed(["ops/ansible/roles/goat-host-firewall/tasks/main.yml"])
        self.assertTrue({"peering-forwarder", "peering-host-firewall"} <= got)

    def test_docs_read_by_tests_are_not_docs(self):
        self.assertIn("go", needed(["docs/operators/roster.json"]))
        self.assertIn("rust", needed(["docs/reference/goatbuilder/command-reference.md"]))

    def test_workflow_change_runs_everything(self):
        self.assertEqual(needed([".github/workflows/ci.yml"]), EVERY)

    def test_classifier_change_runs_everything(self):
        self.assertEqual(needed(["ops/ci/ci_areas.py"]), EVERY)

    def test_unclaimed_path_runs_everything(self):
        plan, reason = ci_areas.plan(["provisioning/new-thing.yaml"])
        self.assertEqual({j for j, n in plan.items() if n}, EVERY)
        self.assertIn("provisioning/new-thing.yaml", reason)

    def test_empty_diff_runs_only_always_on(self):
        self.assertEqual(needed([]), ALWAYS)


if __name__ == "__main__":
    unittest.main()
