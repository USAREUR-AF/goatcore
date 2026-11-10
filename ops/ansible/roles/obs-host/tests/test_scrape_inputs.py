#!/usr/bin/env python3
"""Execute the real read-only Ansible preflight against loopback HTTP fixtures."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
import yaml

TASKS = Path(__file__).resolve().parents[1] / "tasks/verify-scrape-inputs.yml"

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        status, body = self.server.response
        self.send_response(status)
        if status == 302:
            self.send_header("Location", "/metrics")
        self.end_headers()
        self.wfile.write(body.encode())
    def log_message(self, *args):
        pass

class ScrapeInputTests(unittest.TestCase):
    def probe(self, body, status=200, enabled=True, check_mode=False):
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        server.response = (status, body)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                sentinel = root / "changed-config"
                targets = [{"name": "fixture", "addr": "127.0.0.1:%d" % server.server_port,
                            "required_metrics": ["fixture_metric"]}] if enabled else []
                play = [{"hosts": "localhost", "connection": "local", "gather_facts": False,
                         "vars": {"obs_scrape_targets": targets}, "tasks": [
                             {"ansible.builtin.import_tasks": str(TASKS)},
                             {"name": "sentinel configuration change", "ansible.builtin.copy": {"dest": str(sentinel), "content": "changed"}}]}]
                path = root / "play.yml"
                path.write_text(yaml.safe_dump(play))
                env = dict(os.environ, ANSIBLE_LOCAL_TEMP=str(root / "local"),
                           ANSIBLE_REMOTE_TEMP=str(root / "remote"), ANSIBLE_NOCOLOR="1")
                command = ["ansible-playbook", "-i", "localhost,", str(path)]
                if check_mode:
                    command.append("--check")
                result = subprocess.run(command,
                                        env=env, text=True, capture_output=True, timeout=45)
                return result, sentinel.exists()
        finally:
            server.shutdown()
            server.server_close()
            worker.join()

    def test_real_sample_allows_configuration(self):
        result, changed = self.probe('fixture_metric{kind="test"} 1.5\n')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(changed)

    def test_unlabelled_zero_is_a_real_sample(self):
        result, changed = self.probe('fixture_metric 0\n')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(changed)

    def test_empty_html_type_only_prefix_and_invalid_value_refused(self):
        for body in ('', '<html>OK</html>', '# TYPE fixture_metric gauge\n',
                     'fixture_metric_other 1\n', 'fixture_metric BAD\n'):
            with self.subTest(body=body):
                result, changed = self.probe(body)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertFalse(changed)

    def test_consumable_sample_forms_allowed(self):
        # Labelled, multi-label with an escaped quote, exponent, special values
        # and a trailing timestamp are all lines Prometheus's parser accepts.
        for body in ('fixture_metric{kind="test",note="a \\"q\\" b"} 1.5\n',
                     'fixture_metric 1.5e3\n', 'fixture_metric -2E-1\n',
                     'fixture_metric NaN\n', 'fixture_metric +Inf\n',
                     'fixture_metric 1 1700000000000\n', 'fixture_metric{} 3\n'):
            with self.subTest(body=body):
                result, changed = self.probe(body)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertTrue(changed)

    def test_unparseable_sample_lines_refused(self):
        # Prometheus rejects both: invalid label syntax, and a timestamp that
        # is not an integer. Each must stop before the configuration change.
        for body in ('fixture_metric{broken} 1\n', 'fixture_metric 1 INVALID\n',
                     'fixture_metric{kind=test} 1\n', 'fixture_metric 1 2 3\n'):
            with self.subTest(body=body):
                result, changed = self.probe(body)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertFalse(changed)

    def test_failed_check_never_prints_the_response_body(self):
        result, changed = self.probe('review_logging_marker 1\n')
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(changed)
        self.assertNotIn("review_logging_marker", result.stdout + result.stderr)
        self.assertIn("fixture: fixture_metric", result.stdout + result.stderr)

    def test_http_failure_and_redirect_refused_before_config_change(self):
        for status in (404, 302):
            with self.subTest(status=status):
                result, changed = self.probe('fixture_metric 1\n', status=status)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(changed)

    def test_check_mode_still_checks_samples_without_writing(self):
        for body, expected in [('fixture_metric 1\n', 0), ('', 2)]:
            with self.subTest(body=body):
                result, changed = self.probe(body, check_mode=True)
                self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
                self.assertFalse(changed)

    def test_existing_unannotated_targets_unchanged(self):
        result, changed = self.probe('', status=404, enabled=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(changed)

if __name__ == "__main__":
    unittest.main()
