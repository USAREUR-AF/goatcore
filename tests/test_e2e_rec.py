"""ops/e2e/rec.py records every step it is given, including one whose command cannot start."""
import json
import os
import subprocess
import sys
import tempfile

REC = os.path.join(os.path.dirname(__file__), "..", "ops", "e2e", "rec.py")


def run(tmp, *argv):
    env = {**os.environ, "E2E_RECORD_DIR": tmp}
    p = subprocess.run([sys.executable, REC, *argv], env=env, capture_output=True, text=True)
    rows = []
    path = os.path.join(tmp, "report.jsonl")
    if os.path.exists(path):
        rows = [json.loads(line) for line in open(path, encoding="utf-8") if line.strip()]
    return p, rows


def test_a_passing_step_is_recorded():
    with tempfile.TemporaryDirectory() as tmp:
        p, rows = run(tmp, "says-hello", "--expect-in", "hello", "--", "echo", "hello")
        assert p.returncode == 0
        assert [r["status"] for r in rows] == ["pass"]


def test_a_command_that_cannot_start_is_recorded_as_a_failure():
    with tempfile.TemporaryDirectory() as tmp:
        p, rows = run(tmp, "no-such-command", "--", "fe2e-no-such-command-anywhere", "arg")
        assert p.returncode == 1, p.stderr
        assert "Traceback" not in p.stderr
        assert len(rows) == 1
        row = rows[0]
        assert row["name"] == "no-such-command" and row["status"] == "fail"
        assert row["exit"] == 127
        assert "could not start" in row["why"] and "fe2e-no-such-command-anywhere" in row["why"]
        assert os.path.exists(os.path.join(tmp, row["log"]))


def test_a_command_that_cannot_start_is_not_an_expected_failure():
    with tempfile.TemporaryDirectory() as tmp:
        p, rows = run(tmp, "no-such-command", "--expect-exit", "127", "--", "fe2e-no-such-command-anywhere")
        assert p.returncode == 1
        assert rows and rows[0]["status"] == "fail"
