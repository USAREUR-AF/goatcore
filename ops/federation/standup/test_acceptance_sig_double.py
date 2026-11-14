"""The test double of the acceptance-result signer and verifier
(acceptance_sig_double.py) is held to the Rust implementation's committed vector,
byte for byte, so the runner's tests that use it test the rule the command uses."""
import base64
import importlib.util
import json
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
DATA = REPO / "crates/goat-federation/src/testdata/acceptance"
KEYS = REPO / "ops/federation/vectors/testkeys"

spec = importlib.util.spec_from_file_location("acceptance_sig_double", HERE / "acceptance_sig_double.py")
double = importlib.util.module_from_spec(spec)
spec.loader.exec_module(double)

# The fixture's session, as crates/goat-federation/src/acceptance_sig.rs's tests hold it.
FIELDS = {"agreement_sha256": "3d2bd7e59ac1d1cdb4fe6a7b4e3a85f0cba0ddd04a4ab1a63e64ab1a8b7b0f5d",
          "session_id": "ses-xaa-xab-20261109-1", "attempt": 1, "by": "xab", "for": "xaa",
          "echoes": "6f1c2a9e4b7d03e5a8c1f2b3d4e5f607"}


class Double(unittest.TestCase):
    def setUp(self):
        self.result = (DATA / "result-xab.json").read_bytes()
        self.seed = bytes.fromhex((KEYS / "xab.seed.hex").read_text().strip())
        self.keys = {s: base64.b64decode((KEYS / ("%s.public.b64" % s)).read_text().strip()) for s in ("xaa", "xab")}
        self.expect = {"agreement_sha256": FIELDS["agreement_sha256"], "session_id": FIELDS["session_id"],
                       "attempt": 1, "us": "xaa", "them": "xab", "our_challenge": FIELDS["echoes"]}

    def test_the_statement_is_the_rust_vectors_byte_for_byte(self):
        import hashlib
        st = double.statement(FIELDS, hashlib.sha256(self.result).hexdigest(),
                              hashlib.sha256(self.keys["xab"]).hexdigest())
        self.assertEqual(st, (DATA / "statement-xab.txt").read_text())

    def test_the_signature_document_is_the_rust_vectors_byte_for_byte(self):
        doc = double.sign(self.seed, FIELDS, self.result)
        self.assertEqual(doc, (DATA / "signature-xab.json").read_text().rstrip("\n"))

    def test_the_committed_vector_verifies_and_a_stale_echo_or_a_changed_byte_is_refused(self):
        sig = (DATA / "signature-xab.json").read_bytes()
        v = double.verify(self.result, sig, self.keys, self.expect)
        self.assertEqual(v["key_b64"], (KEYS / "xab.public.b64").read_text().strip())
        stale = dict(self.expect, our_challenge="0f1e2d3c4b5a69788796a5b4c3d2e1f0")
        with self.assertRaises(double.Refused) as e:
            double.verify(self.result, sig, self.keys, stale)
        self.assertTrue(str(e.exception).startswith("GB-273: "))
        with self.assertRaises(double.Refused) as e:
            double.verify(self.result + b" ", sig, self.keys, self.expect)
        self.assertTrue(str(e.exception).startswith("GB-272: "))


if __name__ == "__main__":
    unittest.main()
