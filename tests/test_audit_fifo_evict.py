#!/usr/bin/env python3
"""Execute the eviction engine against versioned, mutable fixture state."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import yaml

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("evict", ROOT / "ops/ansible/roles/audit-fifo-evict/files/evict.py")
engine = importlib.util.module_from_spec(spec)
spec.loader.exec_module(engine)
MIB = 2**20


def version(site, hour, size, identity):
    return {"status": "success", "key": f"{site}/2026/01/01/{hour:02d}/event.ndjson",
            "versionId": identity, "size": size}


class Backend:
    def __init__(self, rows, used=972 * MIB):
        self.rows = copy.deepcopy(rows)
        self.ballast = used - sum(row["size"] for row in rows)
        self.removed = []
        self.trails = []
        self.measure_error = self.delete_error = self.trail_error = False

    def used(self, target):
        if self.measure_error:
            raise engine.Failure("fixture measurement failed")
        return self.ballast + sum(row["size"] for row in self.rows)

    def versions(self, target):
        return copy.deepcopy(self.rows)

    def remove(self, target, identity):
        if self.delete_error:
            raise engine.Failure("fixture removal failed")
        matches = [row for row in self.rows if row["versionId"] == identity]
        if len(matches) != 1 or target != "fixture/audit-integrity/" + matches[0]["key"]:
            raise engine.Failure("wrong version target")
        self.removed.append(matches[0])
        self.rows.remove(matches[0])

    def trail(self, source, target):
        if self.trail_error:
            raise engine.Failure("fixture trail failed")
        self.trails.append((json.loads(source.read_text()), target))


class Eviction(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.directory = Path(tmp.name)
        self.config = dict(bucket="audit-integrity", alias="fixture", quota_gib=1,
                           high_pct=90, low_pct=80, batch_hours=6, max_pass_pct=25,
                           trail_prefix="ops/eviction", textfile_dir=tmp.name,
                           state_dir=tmp.name, env_file=tmp.name + "/fixture.env",
                           mc_image="fixture/mc")

    def test_probe_never_lists_or_deletes_at_high_usage(self):
        # Over high-water with eviction disabled: still no listing and no
        # deletion, but the run must now FAIL LOUDLY (exit 4) and raise the
        # over_high_water gauge instead of printing a clean result. The prior
        # contract, returning 0 here, is the silence that let a full bucket
        # refuse every upload while the timer reported success.
        backend = Backend([version("site", 1, 300 * MIB, "v1")])
        backend.versions = lambda target: self.fail("measurement probe must not list versions")
        self.assertEqual(engine.run(self.config, backend), 4)
        self.assertEqual(backend.removed, [])
        self.assertEqual(backend.trails, [])
        prom = (self.directory / "audit_fifo_evict.prom").read_text()
        self.assertIn('goat_audit_evict_over_high_water{bucket="audit-integrity"} 1', prom)

    def test_probe_below_high_water_stays_quiet(self):
        backend = Backend([version("site", 1, 300 * MIB, "v1")], used=800 * MIB)
        backend.versions = lambda target: self.fail("measurement probe must not list versions")
        self.assertEqual(engine.run(self.config, backend), 0)
        prom = (self.directory / "audit_fifo_evict.prom").read_text()
        self.assertIn('goat_audit_evict_over_high_water{bucket="audit-integrity"} 0', prom)

    def test_all_snapshotted_versions_deleted_with_quota_relief(self):
        backend = Backend([version("site", 1, 40 * MIB, "v1"),
                           version("site", 1, 40 * MIB, "v2"),
                           version("site", 2, 80 * MIB, "v3")])
        self.assertEqual(engine.run(self.config, backend, evict=True), 0)
        self.assertEqual({row["versionId"] for row in backend.removed}, {"v1", "v2", "v3"})
        self.assertEqual(backend.used("fixture/audit-integrity"), 812 * MIB)
        self.assertTrue(all(row["confirmed"] for row in backend.trails[-1][0]["attempts"]))

    def test_oversized_oldest_hour_is_not_deleted(self):
        backend = Backend([version("z-site", 1, 300 * MIB, "old"),
                           version("a-site", 2, MIB, "new")])
        self.assertEqual(engine.run(self.config, backend, evict=True), 2)
        self.assertEqual(backend.removed, [])

    def test_preflight_budget_limits_next_irreversible_hour(self):
        backend = Backend([version("site", hour, 150 * MIB, str(hour)) for hour in (1, 2, 3)])
        self.assertEqual(engine.run(self.config, backend, evict=True), 2)
        self.assertEqual([row["versionId"] for row in backend.removed], ["1"])
        self.assertLessEqual(sum(row["size"] for row in backend.removed), 972 * MIB * 25 // 100)

    def test_batch_limit_is_independent_of_byte_limit(self):
        self.config["batch_hours"] = 1
        backend = Backend([version("site", 1, 80 * MIB, "old"), version("site", 2, 80 * MIB, "new")])
        self.assertEqual(engine.run(self.config, backend, evict=True), 2)
        self.assertEqual([row["versionId"] for row in backend.removed], ["old"])

    def test_chronological_order_across_site_prefixes(self):
        self.config["batch_hours"] = 1
        backend = Backend([version("a-site", 2, 80 * MIB, "new"), version("z-site", 1, 80 * MIB, "old")])
        engine.run(self.config, backend, evict=True)
        self.assertEqual([row["versionId"] for row in backend.removed], ["old"])

    def test_measurement_error_is_distinct_from_incomplete(self):
        backend = Backend([])
        backend.measure_error = True
        with self.assertRaises(engine.Failure) as caught:
            engine.run(self.config, backend, evict=True)
        self.assertEqual(caught.exception.code, 1)
        self.assertEqual(backend.removed, [])

    def test_delete_error_is_operational_failure(self):
        backend = Backend([version("site", 1, MIB, "v1")])
        backend.delete_error = True
        with self.assertRaises(engine.Failure) as caught:
            engine.run(self.config, backend, evict=True)
        self.assertEqual(caught.exception.code, 3)
        metrics = (self.directory / "audit_fifo_evict.prom").read_text()
        self.assertIn('goat_audit_evict_last_result{bucket="audit-integrity"} 3', metrics)

    def test_failed_trail_blocks_further_destructive_passes(self):
        backend = Backend([version("site", 1, 80 * MIB, "v1"), version("site", 2, 80 * MIB, "v2")])
        self.config["batch_hours"] = 1
        backend.trail_error = True
        with self.assertRaises(engine.Failure):
            engine.run(self.config, backend, evict=True)
        self.assertTrue((self.directory / "pending-pass.json").exists())
        deleted = len(backend.removed)
        with self.assertRaises(engine.Failure):
            engine.run(self.config, backend, evict=True)
        self.assertEqual(len(backend.removed), deleted)

    def test_duplicate_or_unversioned_listing_refused(self):
        for rows in ([version("site", 1, MIB, "")],
                     [version("site", 1, MIB, "v1")] * 2):
            with self.subTest(rows=rows):
                backend = Backend(rows)
                with self.assertRaises(engine.Failure):
                    engine.run(self.config, backend, evict=True)
                self.assertEqual(backend.removed, [])

    def test_trail_is_excluded_from_data_deletion(self):
        row = {"status": "success", "key": "ops/eviction/2026/01/01/trail.json", "size": MIB,
               "versionId": "trail"}
        backend = Backend([row, version("site", 1, MIB, "data")])
        engine.run(self.config, backend, evict=True)
        self.assertNotIn("trail", [row["versionId"] for row in backend.removed])

    def test_counter_does_not_recount_when_scraped_or_measured(self):
        backend = Backend([version("site", 1, 80 * MIB, "v1"), version("site", 2, 80 * MIB, "v2")])
        engine.run(self.config, backend, evict=True)
        for _ in range(3):
            engine.run(self.config, backend)
        self.assertEqual(json.loads((self.directory / "state.json").read_text())["prefixes_total"], 2)

    def test_invalid_cli_never_returns_bounded_incomplete_status(self):
        result = subprocess.run([sys.executable, str(ROOT / "ops/ansible/roles/audit-fifo-evict/files/evict.py"),
                                 "--unknown-option"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 3)

    def test_command_output_budget_refuses_partial_listing(self):
        command = self.directory / "docker"
        command.write_text("#!/usr/bin/env python3\nprint('x' * 4096)\n")
        command.chmod(0o755)
        with patch.dict(os.environ, {"PATH": str(self.directory) + os.pathsep + os.environ["PATH"]}), \
                patch.object(engine.MC, "OUTPUT_LIMIT", 128):
            with self.assertRaisesRegex(engine.Failure, "bounded output budget"):
                engine.MC(self.config).command("ls", "fixture/audit-integrity")

    def test_installation_probe_precedes_timer_and_only_measures(self):
        tasks = yaml.safe_load((ROOT / "ops/ansible/roles/audit-fifo-evict/tasks/main.yml").read_text())
        probe_index = next(i for i, task in enumerate(tasks) if task["name"] ==
                           "Prove the evictor identity can measure the bucket")
        timer_index = next(i for i, task in enumerate(tasks) if task["name"] ==
                           "Enable and start the evictor timer")
        self.assertLess(probe_index, timer_index)
        self.assertIn("--measure-only", tasks[probe_index]["ansible.builtin.command"]["argv"])
        self.assertEqual(tasks[probe_index]["failed_when"], "_evict_probe.rc != 0")
        defaults = yaml.safe_load((ROOT / "ops/ansible/roles/audit-fifo-evict/defaults/main.yml").read_text())
        self.assertIs(defaults["audit_fifo_evict_enabled"], False)


if __name__ == "__main__":
    unittest.main()
