#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""test_sovereign_pkis.py — the sovereign PKI registry and everything rendered from it.

  registry      schema + the identity rule against each profile's example card
  nginx maps    rendering for three profiles (san_upn, cn_regex, serial); with an
                nginx binary on PATH the maps are run for real: a local nginx with
                mutual TLS, synthetic cards from the test PKI generator, and the
                "<pki>:<identifier>" the maps compose is read back over HTTPS
  claim map     the generated Kratos mapper evaluated with jsonnet for us-dod and
                a non-US profile (needs the `jsonnet` Python package; skipped otherwise)
  generator     the Terraform block for us-dod equals the value collab-services carries

Run: python3 ops/identity/test_sovereign_pkis.py   (PyYAML; jsonnet, nginx, openssl, curl optional)
"""
import copy
import json
import os
import pathlib
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import unittest

import yaml

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import sovereign_pkis as sp  # noqa: E402

REGISTRY = HERE / "sovereign-pkis.yaml"
GRAMMAR = HERE / "grammar.yaml"
GEN_CARD = HERE / "cac-test-pki" / "gen-test-card.sh"
PKI_JS = HERE.parents[1] / "ops/ansible/roles/icam-mtls-proxy/files/pki.js"

# The value collab-services environments/aws/prod/main_cert_signin.tf carries for
# us-dod (its PR regenerates it from this registry). The generator must reproduce it.
LIVE_US_DOD_TERRAFORM = """  default = [
    {
      # U.S. DoD (CAC/PIV): identifier = EDIPI. Generated from USAREUR-AF/goatcore
      # ops/identity/sovereign-pkis.yaml by ops/identity/sovereign-pkis-render.py terraform; edit there.
      id               = "us-dod"
      display          = "U.S. DoD (CAC/PIV)"
      issuer_match     = ",OU=PKI,OU=DoD,O=U\\\\.S\\\\. Government,C=US$"
      identity         = "san_upn"
      identifier_regex = "^(\\\\d{10})(?:\\\\d{6})?@mil$"
      bundle_path      = "pki/us-dod/ca-bundle.pem"
    },
  ]
"""

SERIAL_PROFILE = {
    "id": "xx-serial", "display": "Serial-keyed test PKI", "nation": "XXX", "status": "live",
    "source_scheme": "sovereign-pki", "identifier_name": "certificate serial",
    "issuer_match": "^CN=Serial Test CA,O=Serial Test,C=XX$",
    "identity": {"source": "serial"},
    "subject_cn_format": {"pattern": "<anything>"},
    "trust_anchor": {"publisher": "test", "source_url": "file:///dev/null", "verification": "none", "refresh": "never",
                     "load": "all"},
    "revocation": {"crl": None, "ocsp": None, "loaded": "none"},
    "example": {"identifier": "0A1B", "serial": "0A1B", "subject_dn": "CN=serial user,O=Serial Test,C=XX",
                "issuer_dn": "CN=Serial Test CA,O=Serial Test,C=XX"},
    "sources": [{"what": "test", "url": "n/a", "date": "2026-10-30"}], "unverified": [],
}


def grammar_schemes():
    g = yaml.safe_load(GRAMMAR.read_text(encoding="utf-8"))
    return set(g["dimensions"]["identity.source_scheme"]["values"])


class RegistryTest(unittest.TestCase):
    def setUp(self):
        self.reg = sp.load(REGISTRY)

    def test_registry_is_clean_against_the_grammar(self):
        self.assertEqual(sp.validate(self.reg, grammar_schemes()), [])

    def test_us_dod_is_the_live_profile_and_sketches_render_nowhere(self):
        self.assertEqual([p["id"] for p in sp.live(self.reg)], ["us-dod"])
        for p in sp.pkis(self.reg):
            if p["status"] == "sketch":
                self.assertTrue(p["unverified"], f"{p['id']}: a sketch says what is unverified")

    def test_us_dod_rule_yields_the_edipi_from_both_upn_shapes(self):
        us = sp.by_id(self.reg, "us-dod")
        leaf = sp.leaf_from_example(us["example"])
        leaf["san_upn"] = ["1603452492121002@mil"]        # the shape a live card carries
        self.assertEqual(sp.identifier(us, leaf), "1603452492")
        leaf["san_upn"] = ["1234567890@mil"]              # the older shape
        self.assertEqual(sp.identifier(us, leaf), "1234567890")
        leaf["san_upn"] = ["person-114@example.org"]            # not an EDIPI UPN -> refuse
        self.assertIsNone(sp.identifier(us, leaf))
        leaf["san_upn"] = []
        self.assertIsNone(sp.identifier(us, leaf))

    def test_est_id_rule_reads_the_personal_code_from_the_cn(self):
        est = sp.by_id(self.reg, "est-id")
        self.assertEqual(sp.identifier(est, sp.leaf_from_example(est["example"])), "38001085718")
        self.assertEqual(sp.rfc2253_cn(est["example"]["subject_dn"]), "JÕEORG,JAAK-KRISTJAN,38001085718")

    def test_nationality_is_never_a_registry_field(self):
        for p in sp.pkis(self.reg):
            self.assertNotIn("nationality", json.dumps(p, default=str).lower(), f"{p['id']}: a PKI describes the issuer, never a person")

    def test_validate_refuses_the_mistakes_that_matter(self):
        bad = copy.deepcopy(self.reg)
        bad["pkis"][0]["identity"]["identifier_regex"] = r"^(\d{10})(\d{6})?@mil$"   # two groups
        self.assertTrue(any("exactly one capture group" in p for p in sp.validate(bad)))
        bad = copy.deepcopy(self.reg)
        bad["pkis"][0]["status"] = "sketch"
        bad["pkis"][1]["status"] = "live"                                           # est-id has unverified fields
        self.assertTrue(any("unverified" in p for p in sp.validate(bad)))
        bad = copy.deepcopy(self.reg)
        bad["pkis"].append(copy.deepcopy(bad["pkis"][0]))                           # duplicate id
        self.assertTrue(any("duplicate id" in p for p in sp.validate(bad)))


class NginxMapsTest(unittest.TestCase):
    def setUp(self):
        self.reg = sp.load(REGISTRY)
        self.three = [sp.by_id(self.reg, "us-dod"), sp.by_id(self.reg, "est-id"), SERIAL_PROFILE]

    def test_three_profiles_render_one_chain_each(self):
        maps = sp.nginx_maps(self.three)
        self.assertIn('"~,OU=PKI,OU=DoD,O=U\\.S\\. Government,C=US$" us-dod;', maps)
        self.assertIn('map $pki_san_upn $pki_identifier_us_dod {', maps)
        self.assertIn('"~^(\\d{10})(?:\\d{6})?@mil$" $1;', maps)
        self.assertIn('map $pki_subject_cn $pki_identifier_est_id {', maps)
        self.assertIn('"~(\\d{11})$" $1;', maps)
        self.assertIn('map $ssl_client_serial $pki_identifier_xx_serial {', maps)
        self.assertIn('"~^(.+)$" $1;', maps)
        self.assertIn('map "$pki_id:$pki_identifier" $pki_remote_user {', maps)
        for p in self.three:
            self.assertIn(f"    {p['id']} $pki_identifier_{sp.nginx_var(p['id'])};", maps)
        self.assertEqual(sp.nginx_js_set_lines(self.three),
                         ["js_set $pki_san_upn pki.sanUpn;", "js_set $pki_san_emails pki.sanEmails;"])
        self.assertIsNone(sp.nginx_profiles_js(self.three))

    def test_san_othername_profile_gets_an_njs_wrapper(self):
        other = copy.deepcopy(SERIAL_PROFILE)
        other.update({"id": "xx-other", "identity": {"source": "san_othername", "oid": "1.2.3.4.5"}})
        js = sp.nginx_profiles_js([other])
        self.assertIn("pki.otherName(r, '1.2.3.4.5')", js)
        self.assertIn("js_set $pki_othername_xx_other profiles.othername_xx_other;", sp.nginx_js_set_lines([other]))
        self.assertIn("map $pki_othername_xx_other $pki_identifier_xx_other {", sp.nginx_maps([other]))

    def test_nginx_quote_doubles_backslash_pairs_for_the_config_reader(self):
        self.assertEqual(sp.nginx_quote(r"~^CN=((?:[^,\\]|\\.)*)"), r'"~^CN=((?:[^,\\\\]|\\\\.)*)"')
        self.assertEqual(sp.nginx_quote(r"a\.b"), r'"a\.b"')

    def test_empty_selection_is_refused(self):
        with self.assertRaises(ValueError):
            sp.nginx_maps([])


def _free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


@unittest.skipUnless(shutil.which("nginx") and shutil.which("openssl") and shutil.which("curl"),
                     "nginx, openssl and curl are needed to run the maps for real")
class NginxLiveTest(unittest.TestCase):
    """Run the generated maps in a real nginx with mutual TLS and read back what a
    card resolves to. SAN values come from pki.js when the njs module is present
    (NGINX_JS_MODULE, or the Ubuntu/official-image path); otherwise the test feeds the
    UPN through a header so the regex chain is still the one under test."""

    @classmethod
    def setUpClass(cls):
        cls.reg = sp.load(REGISTRY)
        cls.tmp = pathlib.Path(tempfile.mkdtemp(prefix="sovereign-pki-nginx-"))
        cls.cards = cls.tmp / "cards"
        shutil.copytree(HERE / "cac-test-pki", cls.cards, ignore=shutil.ignore_patterns("out"))
        # gen-test-card.sh reaches the renderer at ../sovereign-pkis-render.py
        for f in ("sovereign-pkis-render.py", "sovereign_pkis.py", "sovereign-pkis.yaml", "grammar.yaml"):
            shutil.copy(HERE / f, cls.tmp / f)
        env = dict(os.environ, PYTHON=sys.executable)
        for pki, ident in (("us-dod", "1603452492"), ("est-id", "38001085718")):
            subprocess.run([str(cls.cards / "gen-test-card.sh"), pki, ident], check=True, env=env,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        # a card from a CA no profile lists
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", str(cls.tmp / "other.key"),
                        "-out", str(cls.tmp / "other.crt"), "-days", "2", "-subj", "/C=YY/O=Some Other PKI/CN=user.other.5555555555"],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", str(cls.tmp / "server.key"),
                        "-out", str(cls.tmp / "server.crt"), "-days", "2", "-subj", "/CN=localhost",
                        "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1"],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        trust = "\n".join((cls.cards / "out" / p / "ca.crt").read_text() for p in ("us-dod", "est-id")) + "\n" + (cls.tmp / "other.crt").read_text()
        (cls.tmp / "trust.pem").write_text(trust)
        cls.js_module = os.environ.get("NGINX_JS_MODULE") or next(
            (m for m in ("/usr/lib/nginx/modules/ngx_http_js_module.so", "/etc/nginx/modules/ngx_http_js_module.so") if os.path.exists(m)), "")
        selected = [sp.by_id(cls.reg, "us-dod"), sp.by_id(cls.reg, "est-id")]
        head = ""
        if cls.js_module:
            shutil.copy(PKI_JS, cls.tmp / "pki.js")
            head = f"load_module {cls.js_module};\n"
            san = f"    js_import pki from {cls.tmp / 'pki.js'};\n    " + "\n    ".join(sp.nginx_js_set_lines(selected)) + "\n"
        else:
            san = ('    map $http_x_test_upn $pki_san_upn { default $http_x_test_upn; }\n'
                   '    map $http_x_test_email $pki_san_emails { default $http_x_test_email; }\n')
        cls.port = _free_port()
        (cls.tmp / "logs").mkdir()
        # pid + error_log inside the temp dir and -e: a packaged nginx (Ubuntu) otherwise
        # opens /run/nginx.pid and /var/log/nginx/error.log, which a test user cannot.
        head += f"pid {cls.tmp / 'nginx.pid'};\nerror_log {cls.tmp / 'logs' / 'error.log'};\n"
        conf = (head + "events {}\nhttp {\n" + san + "\n" + sp.nginx_maps(selected) +
                f"""
    access_log off;
    server {{
        listen 127.0.0.1:{cls.port} ssl;
        ssl_certificate {cls.tmp / 'server.crt'};
        ssl_certificate_key {cls.tmp / 'server.key'};
        ssl_client_certificate {cls.tmp / 'trust.pem'};
        ssl_verify_client optional;
        ssl_verify_depth 3;
        location / {{
            if ($ssl_client_verify != SUCCESS) {{ return 403 "no card"; }}
            if ($pki_id = "") {{ return 403 "not an enabled PKI"; }}
            if ($pki_remote_user = "") {{ return 403 "no identifier"; }}
            return 200 "$pki_remote_user|$pki_id|$pki_san_email";
        }}
    }}
}}
""")
        (cls.tmp / "nginx.conf").write_text(conf)
        cls.nginx = ["nginx", "-e", str(cls.tmp / "logs" / "error.log"), "-c", str(cls.tmp / "nginx.conf"), "-p", str(cls.tmp)]
        r = subprocess.run(cls.nginx + ["-t"], capture_output=True, text=True)
        if r.returncode != 0:
            raise RuntimeError(f"nginx -t failed:\n{r.stderr}\n{(cls.tmp / 'logs' / 'error.log').read_text() if (cls.tmp / 'logs' / 'error.log').exists() else ''}")
        cls.proc = subprocess.Popen(cls.nginx + ["-g", "daemon off;"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(50):
            try:
                socket.create_connection(("127.0.0.1", cls.port), timeout=0.2).close()
                break
            except OSError:
                time.sleep(0.1)

    @classmethod
    def tearDownClass(cls):
        cls.proc.terminate()
        cls.proc.wait(timeout=5)
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def _get(self, cert=None, key=None, headers=()):
        # --tls-max 1.2: macOS's LibreSSL curl aborts a TLS 1.3 CertificateRequest with
        # "decode error"; the maps under test do not depend on the TLS version.
        cmd = ["curl", "-sS", "--tls-max", "1.2", "--cacert", str(self.tmp / "server.crt"), "-o", "-", "-w", "\n%{http_code}",
               f"https://127.0.0.1:{self.port}/auth"]
        if cert:
            cmd += ["--cert", cert, "--key", key]
        for h in headers:
            cmd += ["-H", h]
        out = subprocess.run(cmd, capture_output=True, text=True, check=True).stdout
        body, code = out.rsplit("\n", 1)
        return int(code), body

    def _why(self):
        """The nginx error log, for a failure message (pki.js logs its parse errors there)."""
        log = self.tmp / "logs" / "error.log"
        return "nginx error.log:\n" + (log.read_text()[-3000:] if log.exists() else "(none)")

    def _card(self, pki):
        d = self.cards / "out" / pki
        return str(d / "card.crt"), str(d / "card.key")

    def test_us_dod_card_resolves_to_pki_colon_edipi(self):
        crt, key = self._card("us-dod")
        headers = () if self.js_module else ("X-Test-Upn: 1603452492121002@mil", "X-Test-Email: person-23@example.org")
        code, body = self._get(crt, key, headers)
        self.assertEqual((code, body), (200, "us-dod:1603452492|us-dod|person-23@example.org"), self._why())

    def test_est_id_card_resolves_from_the_subject_cn(self):
        crt, key = self._card("est-id")
        headers = () if self.js_module else ("X-Test-Email: jaak-kristjan.joeorg@eesti.ee",) # oss-allow: public test-card identity
        code, body = self._get(crt, key, headers)
        self.assertEqual((code, body), (200, "est-id:38001085718|est-id|jaak-kristjan.joeorg@eesti.ee"), self._why()) # oss-allow: public test-card identity

    def test_card_from_an_unlisted_issuer_is_refused(self):
        code, body = self._get(str(self.tmp / "other.crt"), str(self.tmp / "other.key"))
        self.assertEqual((code, body), (403, "not an enabled PKI"))

    def test_us_dod_card_whose_upn_is_not_an_edipi_is_refused(self):
        if self.js_module:
            self.skipTest("the UPN comes from the certificate; this case needs the header stub")
        crt, key = self._card("us-dod")
        code, body = self._get(crt, key, ("X-Test-Upn: person-114@example.org",))
        self.assertEqual((code, body), (403, "no identifier"))

    def test_no_card_is_refused(self):
        self.assertEqual(self._get(), (403, "no card"))


class GeneratorTest(unittest.TestCase):
    def setUp(self):
        self.reg = sp.load(REGISTRY)

    def test_terraform_block_equals_the_live_us_dod_value(self):
        self.assertEqual(sp.terraform_value(sp.live(self.reg)), LIVE_US_DOD_TERRAFORM)

    def test_terraform_refuses_a_source_the_front_cannot_read(self):
        other = copy.deepcopy(SERIAL_PROFILE)
        other["identity"] = {"source": "san_othername", "oid": "1.2.3"}
        with self.assertRaises(ValueError):
            sp.terraform_value([other])

    def test_openbao_layout_is_one_dir_per_live_profile(self):
        layout = sp.openbao_layout(sp.live(self.reg))
        self.assertEqual(set(layout), {"us-dod/ca-bundle.pem", "us-dod/SOURCE"})
        self.assertIsNone(layout["us-dod/ca-bundle.pem"])
        self.assertIn("bind-card <name> --pki us-dod", layout["us-dod/SOURCE"])

    def test_committed_claim_map_is_current(self):
        self.assertEqual((HERE / "claim-maps" / "sovereign-pki.jsonnet").read_text(encoding="utf-8"),
                         sp.claim_map(sp.live(self.reg)))

    def test_openssl_request_substitutes_the_identifier_everywhere(self):
        us = sp.by_id(self.reg, "us-dod")
        ca_subj, cnf, info = sp.openssl_request(us, "1603452492")
        self.assertEqual(ca_subj, "/C=US/O=U.S. Government/OU=DoD/OU=PKI/CN=DOD ID CA-81")
        self.assertIn("CN = MAINTAINER.TEST.MI.1603452492", cnf)
        self.assertIn("otherName:msUPN;UTF8:1603452492121002@mil", cnf)
        self.assertEqual(info["identifier"], "1603452492")


def _jsonnet():
    try:
        import _jsonnet  # noqa: F401
        return _jsonnet
    except ImportError:
        return None


@unittest.skipUnless(_jsonnet(), "the jsonnet package is needed to evaluate the claim map")
class ClaimMapTest(unittest.TestCase):
    def setUp(self):
        self.reg = sp.load(REGISTRY)
        self.src = sp.claim_map([sp.by_id(self.reg, "us-dod"), sp.by_id(self.reg, "est-id")])

    def _eval(self, claims):
        out = _jsonnet().evaluate_snippet("sovereign-pki.jsonnet", self.src,
                                          ext_codes={"claims": json.dumps({"raw_claims": claims})})
        return json.loads(out)

    def test_us_dod_binding_carries_pki_subject_and_the_edipi_alias(self):
        got = self._eval({"iss": "https://card-proxy", "sub": "opaque", "preferred_username": "us-dod:1603452492",
                          "email": "us-dod:1603452492"})
        subj = got["identity"]["metadata_public"]["external_subjects"][0]
        self.assertEqual(subj["pki"], "us-dod")
        self.assertEqual(subj["pki_subject"], "1603452492")
        self.assertEqual(subj["edipi"], "1603452492")
        self.assertEqual(subj["subject"], "us-dod:1603452492")
        self.assertEqual(subj["scheme"], "dod-piv")
        self.assertEqual(subj["identifier_name"], "EDIPI")
        self.assertEqual(got["identity"]["traits"]["email"], "1603452492@mil")   # Dex's username-as-email is not an address
        self.assertEqual(got["identity"]["traits"]["groups"], [])
        self.assertNotIn("nationality", json.dumps(got))

    def test_non_us_profile_has_no_edipi_and_keeps_the_cards_email(self):
        got = self._eval({"iss": "https://card-proxy", "sub": "opaque", "preferred_username": "est-id:38001085718",
                          "email": "jaak-kristjan.joeorg@eesti.ee"}) # oss-allow: public test-card identity
        subj = got["identity"]["metadata_public"]["external_subjects"][0]
        self.assertEqual((subj["pki"], subj["pki_subject"], subj["scheme"]), ("est-id", "38001085718", "sovereign-pki"))
        self.assertNotIn("edipi", subj)
        self.assertEqual(got["identity"]["traits"]["email"], "jaak-kristjan.joeorg@eesti.ee") # oss-allow: public test-card identity
        self.assertEqual(got["identity"]["traits"]["name"], "est-id:38001085718")

    def test_non_us_profile_without_an_email_gets_an_undeliverable_one(self):
        got = self._eval({"iss": "x", "sub": "o", "preferred_username": "est-id:38001085718", "email": "est-id:38001085718"})
        self.assertEqual(got["identity"]["traits"]["email"], "38001085718@est-id.card.invalid")

    def test_unknown_profile_or_empty_identifier_mints_nothing(self):
        for user in ("fr-anything:123", "us-dod:", "1603452492"):
            with self.assertRaises(RuntimeError, msg=user):
                self._eval({"iss": "x", "sub": "o", "preferred_username": user})


if __name__ == "__main__":
    unittest.main()
