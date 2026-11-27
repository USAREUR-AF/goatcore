"""The break-glass node's boot-time clock step
(ops/ansible/roles/breakglass-node/templates/breakglass-clock-step.sh.j2),
rendered with the role's defaults and run with stand-ins for htpdate, logger,
systemctl and fake-hwclock, for each answer htpdate 1.3.7 gives. Its exit code
is what the role's converge asserts on, and what the retry timer acts on."""
import subprocess
import tempfile
import unittest
from pathlib import Path

import jinja2
import yaml

REPO = Path(__file__).resolve().parent.parent
ROLE = REPO / "ops/ansible/roles/breakglass-node"


def render(sentinel):
    defaults = yaml.safe_load((ROLE / "defaults/main.yml").read_text())
    text = (ROLE / "templates/breakglass-clock-step.sh.j2").read_text()
    # The role's values hold no character autoescaping changes; escaping keeps CodeQL's rule.
    out = jinja2.Environment(undefined=jinja2.StrictUndefined, autoescape=True).from_string(text).render(**defaults)
    assert "DONE_SENTINEL=/run/breakglass-clock-step.done" in out
    return out.replace("DONE_SENTINEL=/run/breakglass-clock-step.done", f"DONE_SENTINEL={sentinel}")


def run(query, step="exit 0"):
    """The script's exit code, its log, and whether it marked itself done.
    `query` is what htpdate -q does; `step` what htpdate -s does."""
    with tempfile.TemporaryDirectory() as d:
        bin_dir = Path(d, "bin")
        bin_dir.mkdir()
        tools = {
            # The script calls /usr/sbin/htpdate by path; the rendered copy is pointed here.
            "htpdate": f'case "$1" in -q) {query} ;; -s) {step} ;; esac\n',
            "logger": 'shift 3; echo "$*"\n',
            "systemctl": "exit 0\n",
            "fake-hwclock": "exit 0\n",
        }
        for name, body in tools.items():
            p = bin_dir / name
            p.write_text("#!/bin/sh\n" + body)
            p.chmod(0o755)
        sentinel = Path(d, "done")
        script = Path(d, "clock-step.sh")
        script.write_text(render(sentinel).replace("/usr/sbin/htpdate", str(bin_dir / "htpdate")))
        r = subprocess.run(["bash", str(script)], capture_output=True, text=True, timeout=60,
                           env={"PATH": f"{bin_dir}:/usr/bin:/bin"})
        return r.returncode, r.stdout + r.stderr, sentinel.exists()


class ClockStep(unittest.TestCase):
    def test_a_clock_that_needs_no_correction_is_armed_and_done(self):
        # htpdate's answer once its daemon has disciplined the clock.
        rc, log, done = run("echo 'No time correction needed'")
        self.assertEqual((rc, done), (0, True), log)
        self.assertIn("clock plausible (offset 0s", log)

    def test_a_small_offset_is_left_to_the_daemon(self):
        rc, log, done = run("echo 'Offset 0.031 seconds'")
        self.assertEqual((rc, done), (0, True), log)
        self.assertIn("nothing to do", log)

    def test_a_large_offset_is_stepped(self):
        rc, log, done = run("echo 'Offset 86400.000 seconds'")
        self.assertEqual((rc, done), (0, True), log)
        self.assertIn("stepping", log)
        rc, log, done = run("echo 'Offset 86400.000 seconds'", step="exit 1")
        self.assertEqual((rc, done), (1, False), log)
        self.assertIn("step FAILED", log)

    def test_no_reachable_source_is_retried_and_never_done(self):
        for query in ("echo 'No server suitable for synchronization found'; exit 1",
                      "echo 'host or service unavailable'", "exit 0"):
            rc, log, done = run(query)
            self.assertEqual((rc, done), (1, False), f"{query}: {log}")
            self.assertIn("no HTTP time source reachable", log)


if __name__ == "__main__":
    unittest.main()
