#!/usr/bin/env python3
"""ops/preflight-enrollment.sh reads a WireGuard link's state from its UP flag,
and with --key-at-enrolment passes a node that is already enrolled.

A WireGuard link has no carrier, so `ip -br link` reports its operational state
as UNKNOWN even when it is up: an enrolled node's wt0 reads

    wt0   UNKNOWN   <POINTOPOINT,NOARP,UP,LOWER_UP>

join-node phase 9 runs the script with --key-at-enrolment before phase 10, whose
enrolment skips a host already connected; a resume through phase 9 meets an
enrolled node. The script runs here with stand-ins for ip, pgrep, netbird and curl.
"""
import email.utils
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "ops/preflight-enrollment.sh"

ENROLLED = "wt0              UNKNOWN        <POINTOPOINT,NOARP,UP,LOWER_UP> "
DOWN = "wt0              DOWN           <POINTOPOINT,NOARP> "


class Wt0(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.bin = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def stub(self, name, body):
        p = self.bin / name
        p.write_text("#!/bin/sh\n" + body + "\n")
        p.chmod(0o755)

    def run_script(self, wt0=None, netbird_running=True, *args):
        if wt0 is None:
            self.stub("ip", 'exit 1')
        else:
            self.stub("ip", f'case "$1" in -br) echo "{wt0}" ;; *) echo "5: wt0: <...>" ;; esac')
        self.stub("pgrep", "exit 0" if netbird_running else "exit 1")
        self.stub("netbird", 'echo 0.75.1')
        date = email.utils.formatdate(usegmt=True)
        # A curl that answers only when given the CA the management's certificate needs.
        need = getattr(self, "ca_needed", False)
        self.stub("curl", (f'echo "$*" >> {self.bin}/curl.args; ' + (f'for a; do [ -n "$ca" ] && cp "$a" {self.bin}/ca.seen && ca=ok && break; [ "$a" = --cacert ] && ca=1; done; [ "$ca" = ok ] || exit 60; ' if need else '')) + f'case " $* " in *" -sI "*) printf "HTTP/1.1 401 Unauthorized\\r\\nDate: {date}\\r\\n\\r\\n" ;; *) printf 401 ;; esac')
        env = {"PATH": f"{self.bin}:/usr/bin:/bin", "NETBIRD_MANAGEMENT_URL": "https://netbird.zagros.example.org"}
        if getattr(self, "ca", None):
            env["NETBIRD_MANAGEMENT_CA_PEM"] = self.ca
        if getattr(self, "key", None):
            env["NETBIRD_SETUP_KEY"] = self.key
        return subprocess.run(["bash", str(SCRIPT), *args], capture_output=True, text=True, env=env, timeout=60)

    def test_a_management_signed_by_the_goatnets_own_root_is_verified_against_it(self):
        """A LAN site serves the goatnet's own certificates: without the root the
        checks cannot reach management; given it (NETBIRD_MANAGEMENT_CA_PEM),
        both curls verify against that file and the node passes."""
        self.ca_needed = True
        r = self.run_script(None, True, "--key-at-enrolment")
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.ca = "-----BEGIN CERTIFICATE-----\nMIIBfake\n-----END CERTIFICATE-----"
        (self.bin / "curl.args").unlink()
        r = self.run_script(None, True, "--key-at-enrolment")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        calls = (self.bin / "curl.args").read_text().splitlines()
        self.assertEqual(len(calls), 2, calls)
        self.assertTrue(all("--cacert" in c for c in calls), calls)
        self.assertEqual((self.bin / "ca.seen").read_text().strip(), self.ca)

    def test_a_fresh_node_passes(self):
        r = self.run_script(None, True, "--key-at-enrolment")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("no stale wt0 interface", r.stdout)

    def test_an_enrolled_node_passes_before_the_enrolment_that_mints_the_key(self):
        r = self.run_script(ENROLLED, True, "--key-at-enrolment")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("already enrolled", r.stdout)
        self.assertNotIn("stale state", r.stdout)

    def test_an_enrolled_node_still_warns_a_person_with_a_key_in_hand(self):
        self.key = "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE"
        r = self.run_script(ENROLLED, True)
        self.assertEqual(r.returncode, 2, r.stdout)
        self.assertIn("node appears already enrolled", r.stdout)
        self.assertNotIn("stale state", r.stdout)

    def test_a_link_that_is_down_is_stale(self):
        r = self.run_script(DOWN, True, "--key-at-enrolment")
        self.assertEqual(r.returncode, 1, r.stdout)
        self.assertIn("stale state", r.stdout)

    def test_an_up_link_with_no_netbird_is_stale(self):
        r = self.run_script(ENROLLED, False, "--key-at-enrolment")
        self.assertEqual(r.returncode, 1, r.stdout)
        self.assertIn("stale state", r.stdout)


if __name__ == "__main__":
    unittest.main()
