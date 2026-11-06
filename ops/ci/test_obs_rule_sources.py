#!/usr/bin/env python3
"""Exercise remote producer ownership and fail-closed source declarations."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import yaml

GUARD = Path(__file__).with_name("check-obs-rules-have-scrapers.sh").resolve()

class RuleSourceTests(unittest.TestCase):
    def run_guard(self, target=None, remote=True, local=False, gate="collector_enabled", gap=None):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            def write(path, value):
                out = root / path
                out.parent.mkdir(parents=True, exist_ok=True)
                out.write_text(yaml.safe_dump(value))
            write("ops/ansible/roles/obs-host/defaults/main.yml", {"obs_rule_files": ["test.yml"]})
            write("ops/compose/goat-obs/prometheus/rules/test.yml", {"groups": [{"name": "test", "rules": [{"alert": "Test", "expr": "sample_total > 0"}]}]})
            write("ops/pins/obs-metric-sources.yaml", {"sources": [{"prefix": "sample_", "job": "node-exporter", "requires_var": gate}], "declared_gaps": {"site_obs_hosts": {"test.yml": gap}} if gap else {}})
            write("ops/ansible/group_vars/site_hosts/vars.yml", {gate: local})
            write("ops/ansible/group_vars/remote_hosts/vars.yml", {gate: remote})
            write("ops/ansible/group_vars/site_obs_hosts/vars.yml", {"obs_scrape_targets": [target or {"name": "remote", "addr": "192.0.2.10:9100", "job": "node_exporter", "producer_group": "remote_hosts"}]})
            env = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
            return subprocess.run(["bash", str(GUARD)], cwd=root, env=env, text=True, capture_output=True)

    def test_remote_producer_enabled(self):
        result = self.run_guard()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_remote_producer_disabled(self):
        self.assertNotEqual(self.run_guard(remote=False).returncode, 0)

    def test_unannotated_target_keeps_local_gate(self):
        self.assertNotEqual(self.run_guard(target={"name": "local", "job": "node_exporter"}).returncode, 0)
        self.assertEqual(self.run_guard(target={"name": "local", "job": "node_exporter"}, local=True).returncode, 0)

    def test_local_flag_cannot_enable_remote_producer(self):
        self.assertNotEqual(self.run_guard(remote=False, local=True).returncode, 0)

    def test_wrong_job_cannot_supply_gate(self):
        self.assertNotEqual(self.run_guard(target={"name": "wrong", "job": "other", "producer_group": "remote_hosts"}).returncode, 0)

    def test_unknown_producer_group_refused(self):
        result = self.run_guard(target={"name": "bad", "job": "node_exporter", "producer_group": "missing_hosts"}, local=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("producer_group", result.stdout)

    def test_producer_group_path_refused(self):
        self.assertNotEqual(self.run_guard(target={"name": "bad", "job": "node_exporter", "producer_group": "../remote_hosts"}, local=True).returncode, 0)

    def test_obs_timer_cannot_be_supplied_by_remote_host(self):
        self.assertNotEqual(self.run_guard(gate="obs_mesh_pq_enabled").returncode, 0)

    def test_expired_exception_still_fails(self):
        self.assertNotEqual(self.run_guard(remote=False, gap={"reason": "fixture", "until": "2000-01-01"}).returncode, 0)

if __name__ == "__main__":
    unittest.main()
