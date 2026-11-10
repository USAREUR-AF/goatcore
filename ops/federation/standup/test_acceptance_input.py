"""acceptance.sh accepts only well-formed set names, sovereign ids and trust domains
from the agreement, and refuses anything else before it runs a single command.

Run: python3 -m pytest ops/federation/standup/test_acceptance_input.py
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parent / "acceptance.sh"


def agreement(us="xaa", them="xab", ours=("release/all/**",), theirs=("release/all/**",), td=None):
    def flow(frm, to, rel):
        return {"from": frm, "to": to, "publish_under": "from/%s/**" % frm, "release": list(rel),
                "envelope": {"hz": 100, "bytes_per_message": 1048576, "distinct_keys": 5000}}
    return {"spec": "federation-agreement/v1", "expires": "2026-12-01",
            "flows": [flow(us, them, ours), flow(them, us, theirs)],
            "parties": [{"sovereign_id": s, "trust_domain": (td or {}).get(s, "spiffe://%s.example" % s)}
                        for s in (us, them)]}


class Input(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="acceptance-input-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.marker = os.path.join(self.tmp, "ran")

    def run_with(self, ag):
        path = os.path.join(self.tmp, "agreement.json")
        with open(path, "w") as f:
            json.dump(ag, f)
        env = {k: v for k, v in os.environ.items() if not k.startswith("FED_")}
        # Unreachable surfaces: every check that would reach a core fails fast.
        env.update({"FED_OURS_REST": "http://127.0.0.1:9", "FED_THEIRS_REST": "http://127.0.0.1:9"})
        p = subprocess.run(["bash", str(SCRIPT), path, "--as", "xaa"], env=env, capture_output=True,
                           text=True, timeout=300)
        return p.returncode, p.stdout + p.stderr

    def test_malformed_names_are_refused_before_anything_runs(self):
        bad = "$(touch %s)" % self.marker
        cases = {
            "a set name": agreement(ours=("release/all'%s'/**" % bad,)),
            "a partner's set name": agreement(theirs=("release/x;touch %s/**" % self.marker,)),
            "a sovereign id": agreement(them="xab;touch"),
            "a trust domain": agreement(td={"xab": "spiffe://xab.example %s" % bad}),
        }
        for why, ag in cases.items():
            with self.subTest(why=why):
                rc, out = self.run_with(ag)
                self.assertEqual(rc, 2, out)
                self.assertIn("well-formed", out)
                self.assertFalse(os.path.exists(self.marker), "a name from the agreement ran as a command")

    def test_well_formed_names_are_accepted(self):
        rc, out = self.run_with(agreement(ours=("release/all/**", "release/xab.v2/**")))
        self.assertNotEqual(rc, 2, out)
        self.assertNotIn("well-formed", out)


if __name__ == "__main__":
    unittest.main()
