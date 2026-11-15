"""Phase 8 signs an edge leaf's certificate request (gate row B10, slice (b)).

A two-driver peering's edge makes its two keys on the edge host; only their
certificate requests reach the controller. `peering_leaf_material.py --request
<leaf> <file>` signs a request with our CA under the names phase 8 derives, and
refuses (GB-313), before signing either edge leaf, a request that:

- does not carry exactly one subject attribute, CN = the derived name;
- carries any extension or attribute;
- has a key that is not EC P-256;
- is signed with less than SHA-256, or whose self-signature does not verify;
- reuses a key: the other request's, our CA's, another leaf's in the output
  folder, or one recorded as signed before (`--key-record`, which outlives the
  output folder).

Real OpenSSL, a CA and keys made at test time; no key material is committed.
"""
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
HELPER = HERE / "peering_leaf_material.py"
OUT_CN, IN_CN = "fed.xaa.xab.01", "fed.xab.xaa.01"
OUT_SAN, IN_SAN = "DNS:fed.xaa.xab.01,IP:169.254.77.0", "DNS:fed.xab.xaa.01"


def ossl(*args, cwd=None):
    r = subprocess.run(["openssl", *map(str, args)], capture_output=True, cwd=cwd)
    if r.returncode:
        raise AssertionError(f"openssl {args}: {r.stderr.decode()}")
    return r.stdout


def bare_config(d):
    """An OpenSSL config that asks for no extensions, as the edge's request does."""
    p = d / "req.cnf"
    p.write_text("[req]\ndistinguished_name=dn\n[dn]\n")
    return p


class LeafRequests(unittest.TestCase):
    def setUp(self):
        if not shutil.which("openssl"):
            self.skipTest("openssl is required")
        self.d = Path(tempfile.mkdtemp(prefix="leaf-requests-"))
        self.addCleanup(shutil.rmtree, self.d, True)
        self.cnf = bare_config(self.d)
        self.ca, self.ca_key = self.d / "ca.pem", self.d / "ca-key.pem"
        ossl("ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", self.ca_key)
        ossl("req", "-x509", "-new", "-config", self.cnf, "-key", self.ca_key, "-subj", "/CN=xaa test CA",
             "-days", "30", "-addext", "basicConstraints=critical,CA:TRUE",
             "-addext", "keyUsage=critical,keyCertSign,cRLSign", "-out", self.ca)
        der = ossl("x509", "-in", self.ca, "-outform", "DER")
        self.fp = "sha256:" + hashlib.sha256(der).hexdigest()
        self.out = self.d / "pki"
        self.record = self.d / "state" / "edge-leaf-keys.json"

    def key(self, name, kind="ec"):
        k = self.d / f"{name}.key"
        if kind == "ec":
            ossl("ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", k)
        elif kind == "p384":
            ossl("ecparam", "-name", "secp384r1", "-genkey", "-noout", "-out", k)
        elif kind == "rsa":
            ossl("genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out", k)
        elif kind == "ed25519":
            ossl("genpkey", "-algorithm", "ed25519", "-out", k)
        return k

    def request(self, name, key, subj, *extra):
        csr = self.d / f"{name}.csr"
        args = ["req", "-new", "-config", self.cnf, "-key", key, "-subj", subj, "-out", csr, *extra]
        if not any(a.startswith("-sha") for a in extra) and "ed25519" not in str(key):
            args.append("-sha256")
        ossl(*args)
        return csr

    def good(self):
        o = self.request("outward", self.key("outward"), "/CN=" + OUT_CN)
        i = self.request("inward", self.key("inward"), "/CN=" + IN_CN)
        return o, i

    def run_helper(self, outward, inward, *extra):
        cmd = [sys.executable, str(HELPER), "--out", str(self.out), "--ca-cert", str(self.ca),
               "--ca-key", str(self.ca_key), "--ca-fingerprint", self.fp,
               "--leaf", "peering-outward", OUT_CN, OUT_SAN,
               "--leaf", "peering-inward", IN_CN, IN_SAN,
               "--request", "peering-outward", str(outward),
               "--request", "peering-inward", str(inward),
               "--key-record", str(self.record), *extra]
        return subprocess.run(cmd, capture_output=True, text=True)

    def pub(self, path, kind):
        if kind == "csr":
            pem = ossl("req", "-in", path, "-pubkey", "-noout")
        elif kind == "cert":
            pem = ossl("x509", "-in", path, "-pubkey", "-noout")
        else:
            pem = ossl("pkey", "-in", path, "-pubout")
        return subprocess.run(["openssl", "pkey", "-pubin", "-outform", "DER"], input=pem,
                              capture_output=True, check=True).stdout

    def assert_refused(self, r, *words):
        self.assertNotEqual(r.returncode, 0, r.stdout + r.stderr)
        text = r.stdout + r.stderr
        self.assertIn("GB-313", text)
        for w in words:
            self.assertIn(w, text)
        # Nothing signed or written for either edge leaf.
        for leaf in ("peering-outward", "peering-inward"):
            self.assertFalse((self.out / f"{leaf}.pem").exists(), f"{leaf} was signed: {text}")
            self.assertFalse((self.out / f"{leaf}-key.pem").exists())

    def test_a_good_request_is_signed_under_the_derived_names_with_no_key_here(self):
        o, i = self.good()
        r = self.run_helper(o, i)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        for leaf, csr, cn in (("peering-outward", o, OUT_CN), ("peering-inward", i, IN_CN)):
            cert = self.out / f"{leaf}.pem"
            self.assertEqual(self.pub(cert, "cert"), self.pub(csr, "csr"), leaf)
            self.assertFalse((self.out / f"{leaf}-key.pem").exists(), f"{leaf}: a key on the controller")
            subject = ossl("x509", "-in", cert, "-noout", "-subject", "-nameopt", "RFC2253").decode()
            self.assertEqual(subject.strip(), "subject=CN=" + cn)
            text = ossl("x509", "-in", cert, "-noout", "-text").decode()
            self.assertIn("CA:FALSE", text)
            ossl("verify", "-no-CAfile", "-no-CApath", "-trusted", self.ca, cert)
        out_text = ossl("x509", "-in", self.out / "peering-outward.pem", "-noout", "-text").decode()
        self.assertIn("IP Address:169.254.77.0", out_text)
        record = [json.loads(line) for line in self.record.read_text().splitlines()]
        self.assertEqual(sorted(e["leaf"] for e in record), ["peering-inward", "peering-outward"])
        for e in record:
            self.assertTrue(e["public_key_sha256"].startswith("sha256:"), e)
        self.assertTrue((self.out / "our-ca.pem").is_file())

    def test_the_same_requests_again_reuse_the_certificates(self):
        o, i = self.good()
        self.assertEqual(self.run_helper(o, i).returncode, 0)
        before = (self.out / "peering-outward.pem").read_bytes()
        r = self.run_helper(o, i)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("[reused] peering-outward", r.stdout)
        self.assertEqual((self.out / "peering-outward.pem").read_bytes(), before)

    def test_a_request_whose_subject_is_not_exactly_the_derived_cn_is_refused(self):
        i = self.request("inward", self.key("inward"), "/CN=" + IN_CN)
        for name, subj, extra in (
            ("wrong-cn", "/CN=fed.xaa.xac.01", ()),
            ("extra-attribute", f"/O=xaa/CN={OUT_CN}", ()),
            ("multi-valued", f"/CN={OUT_CN}+O=xaa", ("-multivalue-rdn",)),
        ):
            with self.subTest(name):
                o = self.request("outward-" + name, self.key("outward-" + name), subj, *extra)
                self.assert_refused(self.run_helper(o, i), "peering-outward")

    def test_a_request_with_an_extension_or_attribute_is_refused_not_stripped(self):
        i = self.request("inward", self.key("inward"), "/CN=" + IN_CN)
        for name, extra in (
            ("san", ("-addext", "subjectAltName=DNS:" + OUT_CN)),
            ("ca", ("-addext", "basicConstraints=CA:TRUE")),
        ):
            with self.subTest(name):
                o = self.request("outward-" + name, self.key("outward-" + name), "/CN=" + OUT_CN, *extra)
                self.assert_refused(self.run_helper(o, i), "extension")

    def test_a_key_that_is_not_ec_p256_is_refused(self):
        i = self.request("inward", self.key("inward"), "/CN=" + IN_CN)
        for kind in ("rsa", "p384", "ed25519"):
            with self.subTest(kind):
                o = self.request("outward-" + kind, self.key("outward-" + kind, kind), "/CN=" + OUT_CN)
                self.assert_refused(self.run_helper(o, i), "P-256")

    def test_a_weak_or_broken_signature_is_refused(self):
        i = self.request("inward", self.key("inward"), "/CN=" + IN_CN)
        sha1 = self.request("outward-sha1", self.key("outward-sha1"), "/CN=" + OUT_CN, "-sha1")
        self.assert_refused(self.run_helper(sha1, i), "SHA-256")
        good = self.request("outward-good", self.key("outward-good"), "/CN=" + OUT_CN)
        # The same request with another key's public half put in: its
        # self-signature no longer verifies.
        other = self.key("other")
        der = bytearray(ossl("req", "-in", good, "-outform", "DER"))
        old = self.pub(good, "csr")
        new = self.pub(other, "key")
        at = bytes(der).find(old)
        self.assertGreater(at, 0)
        der[at:at + len(old)] = new
        tampered = self.d / "tampered.csr"
        pem = subprocess.run(["openssl", "req", "-inform", "DER", "-outform", "PEM"], input=bytes(der),
                             capture_output=True, check=True).stdout
        tampered.write_bytes(pem)
        self.assert_refused(self.run_helper(tampered, i), "self-signature")

    def test_a_reused_key_is_refused(self):
        shared = self.key("shared")
        o = self.request("outward", shared, "/CN=" + OUT_CN)
        i = self.request("inward", shared, "/CN=" + IN_CN)
        self.assert_refused(self.run_helper(o, i), "reuse")
        i = self.request("inward-ca", self.ca_key, "/CN=" + IN_CN)
        o = self.request("outward-2", self.key("outward-2"), "/CN=" + OUT_CN)
        self.assert_refused(self.run_helper(o, i), "reuse")

    def test_a_key_another_leaf_here_holds_is_refused(self):
        # The core leaf, minted on the controller, then a request with its key.
        r = subprocess.run([sys.executable, str(HELPER), "--out", str(self.out), "--ca-cert", str(self.ca),
                            "--ca-key", str(self.ca_key), "--ca-fingerprint", self.fp,
                            "--leaf", "core", "goat-zenoh", "DNS:goat-zenoh"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        o = self.request("outward", self.out / "core-key.pem", "/CN=" + OUT_CN)
        i = self.request("inward", self.key("inward"), "/CN=" + IN_CN)
        self.assert_refused(self.run_helper(o, i), "reuse")

    def test_a_key_signed_before_is_refused_after_the_output_folder_is_gone(self):
        o, i = self.good()
        self.assertEqual(self.run_helper(o, i).returncode, 0)
        shutil.rmtree(self.out)
        self.assert_refused(self.run_helper(o, i), "reuse")

    def test_a_request_file_holding_a_private_key_is_refused(self):
        o, i = self.good()
        mixed = self.d / "mixed.csr"
        mixed.write_bytes(o.read_bytes() + (self.d / "outward.key").read_bytes())
        self.assert_refused(self.run_helper(mixed, i), "PRIVATE KEY")

    def test_a_ca_with_another_fingerprint_signs_nothing(self):
        o, i = self.good()
        partner = self.d / "partner-ca.pem"
        pk = self.key("partner-ca")
        ossl("req", "-x509", "-new", "-config", self.cnf, "-key", pk, "-subj", "/CN=xab test CA",
             "-days", "30", "-addext", "basicConstraints=critical,CA:TRUE", "-out", partner)
        r = subprocess.run([sys.executable, str(HELPER), "--out", str(self.out), "--ca-cert", str(partner),
                            "--ca-key", str(pk), "--ca-fingerprint", self.fp,
                            "--leaf", "peering-outward", OUT_CN, OUT_SAN,
                            "--request", "peering-outward", str(o), "--key-record", str(self.record)],
                           capture_output=True, text=True)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("fingerprint differs from the agreement", r.stdout + r.stderr)
        self.assertFalse((self.out / "peering-outward.pem").exists())

    def test_phase_8_signs_the_edge_requests_and_mints_the_controllers_own_leaves(self):
        """mint-peering-leaves.sh with --request for both edge leaves: the two edge
        certificates come from the requests, with no key here; the core and the
        republisher are minted here as before. Without --key-record, refused."""
        o, i = self.good()
        agreement = self.d / "agreement.json"
        agreement.write_text(json.dumps({
            "parties": [{"sovereign_id": "xaa", "ca_fingerprint": self.fp},
                        {"sovereign_id": "xab", "ca_fingerprint": "sha256:" + "0" * 64}],
            "transport": {"link_subnet": "169.254.77.0/31"},
        }))
        script = HERE / "mint-peering-leaves.sh"
        base = ["bash", str(script), str(agreement), str(self.out), "--as", "xaa",
                "--ca-cert", str(self.ca), "--ca-key", str(self.ca_key), "--core-san", "192.0.2.10",
                "--request", f"peering-outward={o}", "--request", f"peering-inward={i}"]
        env = {k: v for k, v in os.environ.items() if not k.startswith("FED_")}
        r = subprocess.run(base, capture_output=True, text=True, env=env)
        self.assertNotEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("--key-record", r.stdout + r.stderr)
        self.assertFalse((self.out / "peering-outward.pem").exists())
        r = subprocess.run(base + ["--key-record", str(self.record)], capture_output=True, text=True, env=env)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        for leaf in ("peering-outward", "peering-inward"):
            self.assertTrue((self.out / f"{leaf}.pem").is_file(), leaf)
            self.assertFalse((self.out / f"{leaf}-key.pem").exists(), f"{leaf}: a key on the controller")
        for leaf in ("core", "republisher"):
            self.assertTrue((self.out / f"{leaf}.pem").is_file(), leaf)
            self.assertTrue((self.out / f"{leaf}-key.pem").is_file(), leaf)
        self.assertEqual(self.pub(self.out / "peering-outward.pem", "cert"), self.pub(o, "csr"))


if __name__ == "__main__":
    unittest.main()
