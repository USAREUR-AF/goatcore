"""acceptance.sh against a fabric in one process: it passes only when every check ran
and passed, and a skipped check is never a pass.

Run: python3 -m pytest ops/federation/standup/test_acceptance.py
(or python3 -m unittest from this directory). Needs bash, curl and openssl.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import importlib.util

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "acceptance.sh"

_spec = importlib.util.spec_from_file_location("acceptance_fakefabric", HERE / "acceptance_fakefabric.py")
fakefabric = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fakefabric)

# Keys only this run sets; nothing the developer's shell exports may leak in.
SCRUB = [k for k in os.environ if k.startswith("FED_")]


@unittest.skipUnless(shutil.which("curl") and shutil.which("openssl"), "needs curl and openssl")
class AcceptanceRun(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="acceptance-test-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.ag = fakefabric.agreement()
        self.fabric = fakefabric.Fabric(self.ag, "xaa", self.tmp)
        self.addCleanup(self.fabric.stop)

    def run_acceptance(self, drop=(), extra=None, agreement=None):
        path = os.path.join(self.tmp, "agreement.json")
        with open(path, "w") as f:
            json.dump(agreement or self.ag, f)
        env = {k: v for k, v in os.environ.items() if k not in SCRUB}
        env.update(self.fabric.start())
        for k in drop:
            env.pop(k, None)
        env.update({"FED_ACCEPTANCE_WAIT": "0.3", "FED_ACCEPTANCE_GAP": "0", "FED_ACCEPTANCE_RT_TIMEOUT": "3"})
        env.update(extra or {})
        p = subprocess.run(["bash", str(SCRIPT), path, "--as", "xaa"], env=env,
                           capture_output=True, text=True, timeout=240)
        return p.returncode, p.stdout + p.stderr

    def counts(self, out):
        line = [x for x in out.splitlines() if x.startswith("acceptance-counts:")][-1]
        return dict(kv.split("=", 1) for kv in line.split()[1:])

    def test_everything_works_and_passes(self):
        rc, out = self.run_acceptance()
        c = self.counts(out)
        self.assertEqual((rc, c["fail"], c["skip"]), (0, "0", "0"), out)
        self.assertNotIn("ACCEPTANCE FAILED", out)

    def test_no_counterparty_surface_is_not_a_pass(self):
        # The production shape before this change: only our side, one skip, exit 0.
        rc, out = self.run_acceptance(drop=["FED_THEIRS_REST"])
        c = self.counts(out)
        self.assertEqual(c["fail"], "0", out)
        self.assertNotEqual(c["skip"], "0", out)
        self.assertEqual(rc, 3, out)
        self.assertIn("A skip is not a pass", out)

    def test_strict_off_does_not_turn_a_skip_into_a_pass(self):
        rc, out = self.run_acceptance(drop=["FED_SPIRE_OURS"], extra={"FED_ACCEPTANCE_STRICT": "0"})
        self.assertEqual(rc, 3, out)

    def test_a_missing_trace_is_a_skip_not_a_pass(self):
        rc, out = self.run_acceptance(drop=["FED_EDGE_LOG_OURS"])
        self.assertEqual(rc, 3, out)
        self.assertIn("the ACL trace was not read", out)

    def test_a_failure_still_exits_1(self):
        self.fabric.faults.republisher_down.add("xab")
        rc, out = self.run_acceptance()
        self.assertEqual(rc, 1, out)
        self.assertIn("not delivered inbound", out)

    def test_a_broken_round_trip_fails(self):
        self.fabric.faults.responder_down.add("xab")
        rc, out = self.run_acceptance(extra={})
        self.assertEqual(rc, 1, out)
        self.assertIn("did not complete", out)


if __name__ == "__main__":
    unittest.main()
