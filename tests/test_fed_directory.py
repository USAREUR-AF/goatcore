"""The optional directory step of peering-standup.sh phase 5 (ops/federation/standup/directory.py).

The directory holds pointers, not permissions: a party's entry names where its signed
package is served and that package's sha256, signed by the party. The step publishes
our pointer, looks up the counterparty's, fetches and checks their package, and
reports compatibility. It is advisory: every failure is a warning and exit 0.

A stand-in server plays both the directory (the saltlick write rule: a request
statement signed under the presented key, first write claims the name, offers in a
package-form entry refused with 422) and the static hosts the packages are served
from, so the step runs without a network.
"""
import base64
import contextlib
import hashlib
import http.server
import importlib.util
import io
import json
import shutil
import threading
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[1]
VEC = REPO / "ops/federation/vectors"
spec = importlib.util.spec_from_file_location("fed_directory", REPO / "ops/federation/standup/directory.py")
d = importlib.util.module_from_spec(spec)
spec.loader.exec_module(d)

try:
    import cryptography  # noqa: F401
    HAVE_ED25519 = True
except ImportError:
    HAVE_ED25519 = shutil.which("openssl") is not None
needs_ed25519 = pytest.mark.skipif(not HAVE_ED25519, reason="no Ed25519 implementation (cryptography or openssl)")

XAA = VEC / "compose-full/package-xaa.canonical.json"
XAB = VEC / "compose-full/package-xab.canonical.json"
KEY = {s: VEC / f"testkeys/{s}.seed.hex" for s in ("xaa", "xab")}
PUB = {s: (VEC / f"testkeys/{s}.public.b64").read_text().strip() for s in ("xaa", "xab")}


class Server(http.server.BaseHTTPRequestHandler):
    entries: dict = {}
    files: dict = {}
    refuse_with = None

    def log_message(self, *a):
        pass

    def do_PUT(self):
        sid = self.path.rsplit("/", 1)[-1]
        body = self.rfile.read(int(self.headers["Content-Length"]))
        if Server.refuse_with:
            return self.reply(Server.refuse_with, b"refused")
        doc = json.loads(body)
        if doc.get("spec") == d.PACKAGE_SPEC and any(k in doc for k in d.OFFER_BLOCKS):
            return self.reply(422, b"a directory entry never carries an offer")
        key = self.headers["X-Saltlick-Key"]
        if sid in Server.entries and Server.entries[sid][1] != key:
            return self.reply(403, b"a different key is already bound")
        stmt = d.request_statement("PUT", sid, body, key)
        if not d.verify(base64.b64decode(key), stmt, base64.b64decode(self.headers["X-Saltlick-Signature"])):
            return self.reply(403, b"bad request signature")
        first = sid not in Server.entries
        Server.entries[sid] = (body, key)
        self.reply(201, json.dumps({"first_use": first}).encode())

    def do_GET(self):
        if self.path in Server.files:
            return self.reply(200, Server.files[self.path])
        sid = self.path.rsplit("/", 1)[-1]
        if not self.path.startswith("/v1/parties/") or sid not in Server.entries:
            return self.reply(404, b"not found")
        body, key = Server.entries[sid]
        self.reply(200, body, {"X-Saltlick-Bound-Key": key, "X-Saltlick-Expired": "false"})

    def reply(self, code, body, headers=None):
        self.send_response(code)
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


@pytest.fixture
def server():
    Server.entries, Server.files, Server.refuse_with = {}, {}, None
    srv = http.server.HTTPServer(("127.0.0.1", 0), Server)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    yield f"http://127.0.0.1:{srv.server_address[1]}"
    srv.shutdown()


def run(*args):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        rc = d.main([str(a) for a in args])
    return rc, out.getvalue()


def serve(sid, path):
    Server.files[f"/packages/{sid}.json"] = Path(path).read_bytes()


def test_canonical_form_reproduces_the_conformance_vectors():
    for p in VEC.glob("*/package-*.canonical.json"):
        b = p.read_bytes()
        assert d.canonical(json.loads(b)) == b, p


@needs_ed25519
def test_signing_reproduces_the_vectors_signature():
    pkg = json.loads(XAA.read_bytes())
    seed = d.load_seed(str(KEY["xaa"]))
    assert d.b64(d.public_key(seed)) == PUB["xaa"]
    assert d.b64(d.sign(seed, d.canonical({k: v for k, v in pkg.items() if k != "signature"}))) == pkg["signature"]


@needs_ed25519
def test_publishes_a_signed_pointer_and_nothing_else(server, tmp_path):
    serve("xaa", XAA)
    rc, out = run("--url", server, "--ours", XAA, "--package-url", f"{server}/packages/xaa.json",
                  "--key", KEY["xaa"], "--work", tmp_path)
    assert rc == 0, out
    body, key = Server.entries["xaa"]
    entry = json.loads(body)
    assert sorted(entry) == ["expires", "package_sha256", "package_url", "signature", "sovereign_id", "spec"]
    assert entry["spec"] == d.POINTER_SPEC
    assert entry["package_sha256"] == "sha256:" + hashlib.sha256(XAA.read_bytes()).hexdigest()
    assert key == PUB["xaa"]
    unsigned = {k: v for k, v in entry.items() if k != "signature"}
    assert d.verify(base64.b64decode(key), d.canonical(unsigned), base64.b64decode(entry["signature"]))
    assert (tmp_path / "ours.pointer.json").read_bytes() == body


@needs_ed25519
def test_a_package_url_that_does_not_serve_our_package_publishes_nothing(server):
    serve("xaa", XAB)
    rc, out = run("--url", server, "--ours", XAA, "--package-url", f"{server}/packages/xaa.json", "--key", KEY["xaa"])
    assert rc == 0 and Server.entries == {}
    assert "does not serve our package byte for byte" in out


@needs_ed25519
def test_a_key_that_did_not_sign_the_package_publishes_nothing(server):
    serve("xaa", XAA)
    rc, out = run("--url", server, "--ours", XAA, "--package-url", f"{server}/packages/xaa.json", "--key", KEY["xab"])
    assert rc == 0 and Server.entries == {}
    assert "did not sign our package" in out


@needs_ed25519
def test_looks_up_fetches_checks_and_places_the_counterpartys_package(server, tmp_path):
    serve("xab", XAB)
    run("--url", server, "--ours", XAB, "--package-url", f"{server}/packages/xab.json", "--key", KEY["xab"])
    keyring = tmp_path / "keys.txt"
    keyring.write_text(f"xab={PUB['xab']}\n")
    theirs = tmp_path / "theirs.canonical.json"
    rc, out = run("--url", server, "--ours", XAA, "--their-id", "xab", "--theirs", theirs,
                  "--keyring", keyring, "--work", tmp_path)
    assert rc == 0, out
    assert "PASS    package digest" in out
    assert "PASS    pointer signature" in out and "PASS    package signature" in out
    assert theirs.read_bytes() == XAB.read_bytes()
    assert "COMPATIBILITY REPORT" in out and "the agreement selects `ipsec-fips`" in out
    assert "`xaa` listens" in out


@needs_ed25519
def test_a_package_that_does_not_match_its_pointer_is_not_used(server, tmp_path):
    serve("xab", XAB)
    run("--url", server, "--ours", XAB, "--package-url", f"{server}/packages/xab.json", "--key", KEY["xab"])
    Server.files["/packages/xab.json"] = XAA.read_bytes()
    theirs = tmp_path / "theirs.canonical.json"
    rc, out = run("--url", server, "--ours", XAA, "--their-id", "xab", "--theirs", theirs)
    assert rc == 0
    assert "FAIL    package digest" in out and not theirs.exists()
    assert "COMPATIBILITY REPORT" not in out


@needs_ed25519
def test_without_a_bound_key_the_signatures_are_unknown_not_pass(server, tmp_path):
    serve("xab", XAB)
    run("--url", server, "--ours", XAB, "--package-url", f"{server}/packages/xab.json", "--key", KEY["xab"])
    rc, out = run("--url", server, "--ours", XAA, "--their-id", "xab")
    assert rc == 0 and "UNKNOWN pointer signature" in out and "read their key back by voice" in out


@needs_ed25519
def test_a_refusing_directory_is_a_warning(server):
    serve("xaa", XAA)
    Server.refuse_with = 422
    rc, out = run("--url", server, "--ours", XAA, "--package-url", f"{server}/packages/xaa.json",
                  "--key", KEY["xaa"], "--their-id", "xab")
    assert rc == 0 and "WARN    publish refused, HTTP 422" in out


def test_an_unreachable_directory_is_a_warning():
    rc, out = run("--url", "http://127.0.0.1:9", "--ours", XAA, "--their-id", "xab", "--timeout", "2")
    assert rc == 0
    assert "FED_PACKAGE_URL is not set" in out and "directory unreachable" in out


def entry(sid, td, transports, fips=True, pqc=True, inbound=None):
    caps = {"transports": transports, "fabric_fips_image": fips, "pqc_capable": pqc, "fabric_version": "1.9.0"}
    if inbound:
        caps["inbound"] = inbound
    return {"spec": "federation-package/v1", "sovereign_id": sid, "issued": "2026-09-05", "expires": "2999-01-01",
            "identity": {"trust_domain": td, "ca_fingerprint": "sha256:" + hashlib.sha256(sid.encode()).hexdigest()},
            "capabilities": caps}


def rows(r):
    return {check: (status, detail, fix) for status, check, detail, fix in r.rows}


def test_both_inbound_none_fails_and_names_the_forwarder():
    r = rows(d.compatibility(entry("xaa", "spiffe://a", ["wireguard"], inbound="none"),
                             entry("xab", "spiffe://b", ["wireguard"], inbound="none"), False))
    assert r["inbound"][0] == "FAIL"
    assert "one side must run a peering forwarder before issuing its package (ADR 1218)" in r["inbound"][2]


def test_absent_inbound_and_tool_version_are_unknown_not_pass():
    r = rows(d.compatibility(entry("xaa", "spiffe://a", ["wireguard"]), entry("xab", "spiffe://b", ["wireguard"]), False))
    assert r["inbound"][0] == "UNKNOWN" and r["tool_version"][0] == "UNKNOWN"
    assert r["transports"][0] == "PASS" and "`wireguard`" in r["transports"][1]


def test_refusals_compose_would_make_are_fails():
    r = rows(d.compatibility(entry("xaa", "spiffe://same", ["wireguard"]), entry("xab", "spiffe://same", ["ipsec-fips"]), False))
    assert r["trust_domain"][0] == "FAIL" and r["transports"][0] == "FAIL"
    r = rows(d.compatibility(entry("xaa", "spiffe://a", ["quic"]), entry("xab", "spiffe://b", ["quic"]), False))
    assert r["transports"][0] == "FAIL" and "preference order" in r["transports"][1]


def test_a_one_sided_fips_claim_warns():
    r = rows(d.compatibility(entry("xaa", "spiffe://a", ["wireguard"], fips=True),
                             entry("xab", "spiffe://b", ["wireguard"], fips=False), False))
    assert r["fabric_fips_image"][0] == "WARN" and "one-sided" in r["fabric_fips_image"][2]


def driver(work, **env):
    import os
    import subprocess
    e = {k: v for k, v in os.environ.items() if not k.startswith("FED_")}
    e.update(FED_WORK=str(work), **{k: str(v) for k, v in env.items()})
    return subprocess.run([str(REPO / "ops/scripts/peering-standup.sh"), "xab-peer", "--from", "5", "--to", "5"],
                          env=e, capture_output=True, text=True, timeout=120)


def test_driver_phase_5_without_a_directory_runs_no_directory_step(tmp_path):
    r = driver(tmp_path)
    assert r.returncode == 0, r.stderr
    assert "Optional: set FED_DIRECTORY_URL" in r.stdout
    assert "Directory step (optional)" not in r.stdout
    assert "OPERATOR STEP" in r.stdout


@needs_ed25519
def test_driver_phase_5_with_a_directory_publishes_and_reports(server, tmp_path):
    serve("xab", XAB)
    run("--url", server, "--ours", XAB, "--package-url", f"{server}/packages/xab.json", "--key", KEY["xab"])
    (tmp_path / "ours.canonical.json").write_bytes(XAA.read_bytes())
    serve("xaa", XAA)
    r = driver(tmp_path, FED_DIRECTORY_URL=server, FED_PACKAGE_URL=f"{server}/packages/xaa.json",
               FED_COUNTERPARTY_ID="xab", FED_SIGNING_KEY=KEY["xaa"])
    assert r.returncode == 0, r.stderr
    assert "published `xaa`" in r.stdout and "COMPATIBILITY REPORT" in r.stdout
    assert (tmp_path / "theirs.canonical.json").read_bytes() == XAB.read_bytes()
    assert r.stdout.rstrip().splitlines()[-1].strip().startswith("^ OPERATOR STEP")


def test_driver_phase_5_with_a_dead_directory_still_stops_cleanly(tmp_path):
    (tmp_path / "ours.canonical.json").write_bytes(XAA.read_bytes())
    r = driver(tmp_path, FED_DIRECTORY_URL="http://127.0.0.1:9", FED_COUNTERPARTY_ID="xab")
    assert r.returncode == 0, r.stderr
    assert "WARN" in r.stdout and "OPERATOR STEP" in r.stdout
