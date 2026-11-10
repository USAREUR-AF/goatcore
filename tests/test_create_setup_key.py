"""Exercise the real shell entry point against a fake aws-vault executable."""

import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest
import unittest.mock
import uuid


# The script takes its aws-vault profile from the caller; the shim ignores it.
os.environ.setdefault("AWS_VAULT_PROFILE", "example")
ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "ops/ansible/scripts/create-setup-key.sh"
spec = importlib.util.spec_from_file_location("setup_key", SCRIPT.with_suffix(".py"))
MODULE = importlib.util.module_from_spec(spec)
spec.loader.exec_module(MODULE)
COMMAND_ID = "12345678-1234-1234-1234-123456789abc"
BASE = ["--site", "vega", "--role", "admin", "--ssm-instance", "i-0123456789abcdef0",
        "--ssm-region", "eu-central-1"]


class SetupKeyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="goat-setup-key-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        shim = self.root / "aws-vault"
        shim.write_text("""#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(os.environ['SETUP_KEY_FIXTURE'])
argv = sys.argv[1:]
with (root / 'calls.jsonl').open('a') as log:
    log.write(json.dumps(argv) + '\\n')
if 'send-command' in argv:
    location = argv[argv.index('--parameters') + 1]
    if location != 'file:///dev/stdin':
        print('refuse credential temp file', file=sys.stderr)
        sys.exit(3)
    payload = json.load(sys.stdin)
    (root / 'payload.json').write_text(json.dumps(payload))
    if os.environ.get('FAKE_SEND_ERROR'):
        print('DO-NOT-PRINT-PROVIDER-SECRET', file=sys.stderr)
        sys.exit(1)
    print(os.environ.get('FAKE_COMMAND_ID', '12345678-1234-1234-1234-123456789abc'))
else:
    print(os.environ.get('FAKE_SSM_STATUS', 'Success'))
""")
        shim.chmod(0o755)
        self.env = {**os.environ, "PATH": str(self.root) + os.pathsep + os.environ["PATH"],
                    "SETUP_KEY_FIXTURE": str(self.root)}

    def invoke(self, extra=(), base=BASE, **env):
        return subprocess.run([os.environ.get("BASH_UNDER_TEST", "/bin/bash"), str(SCRIPT),
                               *base, *extra], env={**self.env, **env}, text=True,
                              capture_output=True, timeout=15)

    def payload(self):
        payload = json.loads((self.root / "payload.json").read_text())
        command = payload["commands"][0]
        encoded = shlex.split(command.splitlines()[1])[2]
        sql = base64.b64decode(encoded, validate=True).decode()
        return payload, command, sql

    def test_actual_cli_groups_hash_mask_and_one_time_output(self):
        result = self.invoke(["--groups", "vega-INFRA,g2"])
        self.assertEqual(result.returncode, 0, result.stderr)
        output = json.loads(result.stdout)
        key = output["key_value"]
        self.assertEqual(str(uuid.UUID(key)).upper(), key)
        payload, command, sql = self.payload()
        self.assertNotIn(key, json.dumps(payload))
        expected_hash = base64.b64encode(hashlib.sha256(key.encode()).digest()).decode()
        self.assertIn(MODULE.sql_literal(expected_hash), sql)
        self.assertIn(MODULE.sql_literal(key[:5] + "****"), sql)
        self.assertIn(MODULE.sql_literal('["vega-INFRA", "g2"]'), sql)
        self.assertIn("ON_ERROR_STOP=1", command)
        self.assertTrue(sql.startswith("BEGIN;\n"))
        self.assertTrue(sql.endswith("COMMIT;\n"))
        self.assertIn("no NetBird account exists", sql)
        self.assertIn(MODULE.sql_literal(output["expires_at"]) + "::timestamptz", sql)
        self.assertNotIn(key, (self.root / "calls.jsonl").read_text())
        self.assertEqual(output["ttl_days"], 7)
        self.assertEqual(output["usage_limit"], 3)

    def test_no_groups_means_empty_json_array_not_os_groups(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("E'[]'", self.payload()[2])

    def test_role_defaults_and_key_type(self):
        for role, ttl, usage in (("bootstrap", 1, 1), ("admin", 7, 3),
                                 ("operator", 30, 25), ("peer", 7, 1)):
            with self.subTest(role=role):
                result = self.invoke(base=[*BASE[:3], role, *BASE[4:]])
                self.assertEqual(result.returncode, 0, result.stderr)
                output = json.loads(result.stdout)
                self.assertEqual((output["ttl_days"], output["usage_limit"]), (ttl, usage))
                self.assertIn(MODULE.sql_literal("one-off" if role == "peer" else "reusable"),
                              self.payload()[2])

    def test_notes_and_groups_remain_data_across_json_shell_and_sql(self):
        note = "O'Brien \\path\nEOF\n$(touch should-not-exist)\n\\! touch bad"
        group = "group'\\\";$(touch also-bad)"
        result = self.invoke(["--note", note, "--groups", group])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["note"], note)
        _, command, sql = self.payload()
        self.assertIn(MODULE.sql_literal(note), sql)
        self.assertIn(MODULE.sql_literal(json.dumps([group])), sql)
        # Execute the actual command with a fake Docker and only the audit
        # destination redirected into the fixture. No database or fleet call.
        docker = self.root / "docker"
        docker.write_text("#!/bin/sh\ncat > \"$SETUP_KEY_FIXTURE/sql.txt\"\n")
        docker.chmod(0o755)
        audit = self.root / "audit.jsonl"
        command = command.replace("/var/log/setup-key-audit.log", shlex.quote(str(audit)))
        run = subprocess.run(["/bin/sh", "-c", command], env=self.env, cwd=self.root,
                             text=True, capture_output=True, timeout=10)
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual((self.root / "sql.txt").read_text(), sql)
        record = json.loads(audit.read_text())
        self.assertEqual(record["note"], note)
        self.assertEqual(record["auto_groups"], [group])
        self.assertNotIn("key_value", record)
        self.assertFalse((self.root / "should-not-exist").exists())
        self.assertFalse((self.root / "also-bad").exists())
        self.assertFalse((self.root / "bad").exists())

    def test_failed_database_command_does_not_write_success_audit(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        docker = self.root / "docker"
        docker.write_text("#!/bin/sh\nexit 3\n")
        docker.chmod(0o755)
        audit = self.root / "audit.jsonl"
        command = self.payload()[1].replace("/var/log/setup-key-audit.log", shlex.quote(str(audit)))
        run = subprocess.run(["/bin/sh", "-c", command], env=self.env, capture_output=True)
        self.assertNotEqual(run.returncode, 0)
        self.assertFalse(audit.exists())

    def test_invalid_inputs_are_refused_before_aws(self):
        for extra in (["--ttl-days", "0"], ["--ttl-days", "1;DROP"],
                      ["--ttl-days", "90"], ["--ttl-days", "2147483647", "--note", "test"],
                      ["--usage-limit", "0"], ["--usage-limit", "-1"],
                      ["--usage-limit", "2147483648"], ["--site", "';DROP"],
                      ["--ssm-instance", "not-an-instance"], ["--ssm-region", "bad"],
                      ["--groups"]):
            with self.subTest(extra=extra):
                result = self.invoke(extra)
                self.assertEqual(result.returncode, 2)
                self.assertFalse((self.root / "calls.jsonl").exists())

    def test_longer_ttl_requires_and_records_reason(self):
        result = self.invoke(["--ttl-days", "90", "--note", "issued before deployment"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["ttl_days"], 90)
        self.assertIn(MODULE.sql_literal("issued before deployment"), self.payload()[2])

    def test_provider_failure_never_echoes_diagnostics_or_key(self):
        result = self.invoke(FAKE_SEND_ERROR="1")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertNotIn("DO-NOT-PRINT-PROVIDER-SECRET", result.stderr)
        calls = (self.root / "calls.jsonl").read_text().splitlines()
        self.assertEqual(len(calls), 1)

    def test_ssm_failures_do_not_print_one_time_key(self):
        for status in ("Failed", "TimedOut", "Cancelled"):
            with self.subTest(status=status):
                result = self.invoke(FAKE_SSM_STATUS=status)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, "")
                self.assertIn(COMMAND_ID, result.stderr)
                self.assertIn("check its outcome before retrying", result.stderr)

    def test_invalid_command_id_is_not_echoed_or_polled(self):
        result = self.invoke(FAKE_COMMAND_ID="DO-NOT-PRINT-PROVIDER-SECRET")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertNotIn("DO-NOT-PRINT-PROVIDER-SECRET", result.stderr)
        self.assertEqual(len((self.root / "calls.jsonl").read_text().splitlines()), 1)

    def test_transient_invocation_failure_and_unknown_status_can_recover(self):
        args = MODULE.arguments(BASE)
        with unittest.mock.patch.object(MODULE, "aws_call", side_effect=[COMMAND_ID, RuntimeError(),
                              "DO-NOT-PRINT-PROVIDER-SECRET", "Success"]), \
                unittest.mock.patch.object(MODULE.time, "sleep") as sleep:
            output = MODULE.run(args)
        self.assertEqual(output["role"], "admin")
        self.assertEqual(sleep.call_count, 2)

    def test_unknown_status_eventually_fails_without_echoing_provider_data(self):
        args = MODULE.arguments(BASE)
        with unittest.mock.patch.object(MODULE, "aws_call", side_effect=[COMMAND_ID] +
                              ["DO-NOT-PRINT-PROVIDER-SECRET"] * 30), \
                unittest.mock.patch.object(MODULE.time, "sleep"), self.assertRaises(RuntimeError) as error:
            MODULE.run(args)
        self.assertIn("not confirmed", str(error.exception))
        self.assertNotIn("DO-NOT-PRINT-PROVIDER-SECRET", str(error.exception))

    def test_missing_cli_and_timeout_have_safe_diagnostics(self):
        for failure in (FileNotFoundError("DO-NOT-PRINT-PROVIDER-SECRET"),
                        subprocess.TimeoutExpired("DO-NOT-PRINT-PROVIDER-SECRET", 60)):
            with self.subTest(failure=type(failure).__name__), \
                    unittest.mock.patch.object(MODULE.subprocess, "run", side_effect=failure), \
                    self.assertRaises(RuntimeError) as error:
                MODULE.aws_call(["send-command"], "DO-NOT-PRINT-PAYLOAD")
            self.assertNotIn("DO-NOT-PRINT", str(error.exception))

    def test_send_timeout_keeps_key_id_for_reconciliation_without_retrying(self):
        with unittest.mock.patch.object(MODULE, "aws_call", side_effect=RuntimeError("AWS command failed")) as aws, \
                self.assertRaises(RuntimeError) as error:
            MODULE.run(MODULE.arguments(BASE))
        self.assertIn("check SSM history for key sk_vega_admin_", str(error.exception))
        self.assertEqual(aws.call_count, 1)


if __name__ == "__main__":
    unittest.main()
