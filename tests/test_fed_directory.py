"""The optional directory step of peering-standup.sh phase 5 (ops/federation/standup/directory.py).

The directory holds pointers, not permissions: a party's entry names where its signed
package is served and that package's sha256, signed by the party. The step publishes
our pointer, looks up the counterparty's, fetches and checks their package, and
reports compatibility. It is advisory: every failure is a warning and exit 0.

A stand-in server plays both the directory (the saltlick write rule: a request
statement signed under the presented key, first write claims the name, offers in a
package-form entry refused with 422; a hosted package or CA needs an entry and the
entry's key) and the static hosts the packages are served from, so the step runs
without a network.
"""
import base64
import contextlib
import hashlib
import http.server
import importlib.util
import io
import json
import os
import shutil
import ssl
import subprocess
import threading
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[1]
VEC = REPO / "ops/federation/vectors"
spec = importlib.util.spec_from_file_location("fed_directory", REPO / "ops/federation/standup/directory.py")
d = importlib.util.module_from_spec(spec)
spec.loader.exec_module(d)

HAVE_ED25519 = importlib.util.find_spec("cryptography") is not None or shutil.which("openssl") is not None
needs_ed25519 = pytest.mark.skipif(not HAVE_ED25519, reason="no Ed25519 implementation (cryptography or openssl)")

XAA = VEC / "compose-full/package-xaa.canonical.json"
XAB = VEC / "compose-full/package-xab.canonical.json"
KEY = {s: VEC / f"testkeys/{s}.seed.hex" for s in ("xaa", "xab")}
PUB = {s: (VEC / f"testkeys/{s}.public.b64").read_text().strip() for s in ("xaa", "xab")}


class Server(http.server.BaseHTTPRequestHandler):
    entries: dict = {}
    files: dict = {}
    hosted: dict = {}
    signed: dict = {}
    refuse_with = None
    no_slots = False  # an older saltlick: no /packages/<counterparty> route
    fail_slot_reads = None  # an HTTP status every GET of a counterparty slot answers with
    fail_reads: dict = {}  # path below /v1/parties/ -> how many GETs of it answer 503 first
    bound_override: dict = {}  # sovereign_id -> the bound key a dishonest directory serves instead
    fail_puts: dict = {}  # path below /v1/parties/ -> how many PUTs of it answer 503 first
    refuse_slot_with = None  # an HTTP status every PUT of a counterparty slot answers with

    def log_message(self, *a):
        pass

    def do_PUT(self):
        body = self.rfile.read(int(self.headers["Content-Length"]))
        rest = self.path.removeprefix("/v1/parties/")
        if Server.fail_puts.get(rest):
            Server.fail_puts[rest] -= 1
            code, reply = 503, b"unavailable"
        elif Server.refuse_slot_with and "/packages/" in rest:
            code, reply = Server.refuse_slot_with, b"withdrawn or superseded"
        elif Server.no_slots and "/packages/" in rest:
            code, reply = 404, b"404 page not found"
        elif "/" in rest:
            code, reply = self.put_hosted(rest, body)
        else:
            code, reply = self.put(rest, body)
        self.reply(code, reply)

    def put_hosted(self, resource, body):
        sid, kind = resource.split("/", 1)
        if sid not in Server.entries:
            return 409, b"publish the entry first"
        key = self.headers["X-Saltlick-Key"]
        if Server.entries[sid][1] != key:
            return 403, b"not the bound key"
        stmt = d.request_statement("PUT", resource, body, key)
        if not d.verify(base64.b64decode(key), stmt, base64.b64decode(self.headers["X-Saltlick-Signature"])):
            return 403, b"bad request signature"
        Server.hosted[resource] = body
        Server.signed[resource] = (key, self.headers["X-Saltlick-Signature"])
        return 201, b"{}"

    def put(self, sid, body):
        if Server.refuse_with:
            return Server.refuse_with, b"refused"
        doc = json.loads(body)
        if doc.get("spec") == d.POINTER_SPEC and not str(doc.get("package_url", "")).startswith("https://"):
            return 422, b"package_url must be https"
        if doc.get("spec") == d.PACKAGE_SPEC and any(k in doc for k in d.OFFER_BLOCKS):
            return 422, b"a directory entry never carries an offer"
        key = self.headers["X-Saltlick-Key"]
        if sid in Server.entries and Server.entries[sid][1] != key:
            return 403, b"a different key is already bound"
        stmt = d.request_statement("PUT", sid, body, key)
        if not d.verify(base64.b64decode(key), stmt, base64.b64decode(self.headers["X-Saltlick-Signature"])):
            return 403, b"bad request signature"
        first = sid not in Server.entries
        Server.entries[sid] = (body, key)
        return 201, json.dumps({"first_use": first}).encode()

    def do_GET(self):
        sid = self.path.rsplit("/", 1)[-1]
        rest = self.path.removeprefix("/v1/parties/")
        if Server.fail_reads.get(rest):
            Server.fail_reads[rest] -= 1
            self.reply(503, b"unavailable")
        elif Server.no_slots and "/packages/" in rest:
            self.reply(404, b"404 page not found")
        elif Server.fail_slot_reads and "/packages/" in rest:
            self.reply(Server.fail_slot_reads, b"unavailable")
        elif rest in Server.hosted:
            owner = rest.split("/")[0]
            headers = {"X-Saltlick-Bound-Key": Server.bound_override.get(owner, Server.entries[owner][1])}
            if rest in Server.signed:  # a file stored without an upload signature is served without one
                key, sig = Server.signed[rest]
                headers.update({"X-Saltlick-Key": key, "X-Saltlick-Signature": sig, "X-Saltlick-Resource": rest})
            self.reply(200, Server.hosted[rest], headers)
        elif self.path in Server.files:
            self.reply(200, Server.files[self.path])
        elif self.path.startswith("/v1/parties/") and "/" not in rest and sid in Server.entries:
            body, key = Server.entries[sid]
            self.reply(200, body, {"X-Saltlick-Bound-Key": key, "X-Saltlick-Expired": "false"})
        else:
            self.reply(404, b"not found")

    def reply(self, code, body, headers=None):
        self.send_response(code)
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


@pytest.fixture(scope="session")
def tls_cert(tmp_path_factory):
    """A throwaway certificate for 127.0.0.1: the directory is https only, as saltlick is."""
    if not shutil.which("openssl"):
        pytest.skip("no openssl to make a test certificate")
    d_ = tmp_path_factory.mktemp("tls")
    crt, key = d_ / "dir.pem", d_ / "dir.key"
    subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1",
                    "-nodes", "-days", "1", "-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1",
                    "-keyout", str(key), "-out", str(crt)], check=True, capture_output=True)
    return crt, key


@pytest.fixture
def server(tls_cert, monkeypatch):
    Server.entries, Server.files, Server.hosted, Server.signed, Server.refuse_with = {}, {}, {}, {}, None
    Server.no_slots, Server.fail_slot_reads, Server.fail_reads, Server.bound_override = False, None, {}, {}
    Server.fail_puts, Server.refuse_slot_with = {}, None
    crt, key = tls_cert
    srv = http.server.HTTPServer(("127.0.0.1", 0), Server)
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(str(crt), str(key))
    srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
    monkeypatch.setenv("SSL_CERT_FILE", str(crt))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    yield f"https://127.0.0.1:{srv.server_address[1]}"
    srv.shutdown()


def refused(path):
    """The one refused copy of path (<path>.refused-<UTC time>); fails if there is not exactly one."""
    got = sorted(Path(path).parent.glob(Path(path).name + ".refused-*"))
    assert len(got) == 1, got
    return got[0]


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
    rc, out = run("--url", "https://127.0.0.1:9", "--ours", XAA, "--their-id", "xab", "--timeout", "2")
    assert rc == 0
    assert "FED_SIGNING_KEY is not set" in out and "directory unreachable" in out


def test_a_plain_http_directory_publishes_nothing():
    rc, out = run("--url", "http://127.0.0.1:9", "--ours", XAA, "--their-id", "xab", "--timeout", "2")
    assert rc == 0
    assert "FAIL    the directory URL is not https" in out


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


def test_driver_phase_5_without_a_directory_names_it_as_required(tmp_path):
    r = driver(tmp_path)
    assert r.returncode == 0, r.stderr
    assert "Set FED_DIRECTORY_URL, FED_COUNTERPARTY_ID and FED_SIGNING_KEY; phase 1 refuses" in r.stdout
    assert "Directory step, run before this stop" not in r.stdout
    assert "OPERATOR STEP" in r.stdout


def test_driver_phase_5_with_no_directory_names_the_files_to_hand_over(tmp_path):
    r = driver(tmp_path, FED_NO_DIRECTORY=1)
    assert r.returncode == 0, r.stderr
    assert "exchange packages by hand (FED_NO_DIRECTORY=1)" in r.stdout
    assert "Obtain theirs and place them at:" in r.stdout
    assert "Directory step, run before this stop" not in r.stdout


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
    r = driver(tmp_path, FED_DIRECTORY_URL="https://127.0.0.1:9", FED_COUNTERPARTY_ID="xab")
    assert r.returncode == 0, r.stderr
    assert "WARN" in r.stdout and "OPERATOR STEP" in r.stdout


# ---------------------------------------------------------------------------
# A party with no public host: the directory hosts its package and CA.
# ---------------------------------------------------------------------------

needs_openssl = pytest.mark.skipif(shutil.which("openssl") is None, reason="no openssl CLI to mint a test CA")


def mint_ca(tmp_path, name):
    """A self-signed CA certificate, PEM, and its `sha256:` fingerprint over the DER."""
    key, crt = tmp_path / f"{name}.key", tmp_path / f"{name}.crt"
    subprocess.run(["openssl", "req", "-x509", "-newkey", "ed25519", "-nodes", "-subj", f"/CN={name} test CA",
                    "-days", "2", "-keyout", str(key), "-out", str(crt),
                    "-addext", "basicConstraints=critical,CA:TRUE"], check=True, capture_output=True)
    pem = crt.read_bytes()
    return pem, d.sha256_of(d.pem_certificates(pem)[0])


def package_with_ca(tmp_path, sid, fingerprint):
    """The conformance package for sid, its ca_fingerprint replaced and re-signed by sid's test key."""
    src = XAA if sid == "xaa" else XAB
    pkg = {k: v for k, v in json.loads(src.read_bytes()).items() if k != "signature"}
    pkg["identity"] = dict(pkg["identity"], ca_fingerprint=fingerprint)
    seed = d.load_seed(str(KEY[sid]))
    raw = d.canonical(dict(pkg, signature=d.b64(d.sign(seed, d.canonical(pkg)))))
    p = tmp_path / f"{sid}.canonical.json"
    p.write_bytes(raw)
    return p


@needs_ed25519
@needs_openssl
def test_without_a_package_url_the_directory_hosts_our_package_and_ca(server, tmp_path):
    ca, fp = mint_ca(tmp_path, "xaa")
    other, _ = mint_ca(tmp_path, "other")
    bundle = tmp_path / "bundle.pem"
    bundle.write_bytes(other + ca)
    ours = package_with_ca(tmp_path, "xaa", fp)
    rc, out = run("--url", server, "--ours", ours, "--key", KEY["xaa"], "--ca", bundle, "--work", tmp_path)
    assert rc == 0, out
    pointer = json.loads(Server.entries["xaa"][0])
    assert pointer["package_url"] == f"{server}/v1/parties/xaa/package"
    assert pointer["package_sha256"] == d.sha256_of(ours.read_bytes())
    assert Server.hosted["xaa/package"] == ours.read_bytes()
    # Only the anchor the package names, never the rest of the bundle.
    assert d.pem_certificates(Server.hosted["xaa/ca"]) == d.pem_certificates(ca)
    assert "uploaded `xaa`'s package to the directory" in out
    assert f"our CA fingerprint, to read aloud: {d.fingerprint_groups(fp)}" in out
    assert "refused" not in out and "not uploaded" not in out


@needs_ed25519
@needs_openssl
def test_the_ca_defaults_to_the_profiles_ca_roots(server, tmp_path, monkeypatch):
    ca, fp = mint_ca(tmp_path, "xaa")
    ctx = tmp_path / "goat/contexts/default"
    ctx.mkdir(parents=True)
    (ctx / "ca-roots.pem").write_bytes(ca)
    monkeypatch.setenv("GOAT_PROFILE_DIR", str(tmp_path / "goat"))
    monkeypatch.delenv("GOAT_CONTEXT", raising=False)
    rc, out = run("--url", server, "--ours", package_with_ca(tmp_path, "xaa", fp), "--key", KEY["xaa"])
    assert rc == 0 and d.pem_certificates(Server.hosted["xaa/ca"]) == d.pem_certificates(ca), out


@needs_ed25519
@needs_openssl
def test_a_ca_that_is_not_the_packages_anchor_is_not_uploaded(server, tmp_path, monkeypatch):
    monkeypatch.setenv("GOAT_PROFILE_DIR", str(tmp_path / "nowhere"))
    _, fp = mint_ca(tmp_path, "xaa")
    wrong, _ = mint_ca(tmp_path, "wrong")
    (tmp_path / "wrong.pem").write_bytes(wrong)
    ours = package_with_ca(tmp_path, "xaa", fp)
    rc, out = run("--url", server, "--ours", ours, "--key", KEY["xaa"], "--ca", tmp_path / "wrong.pem")
    assert rc == 0 and "xaa/ca" not in Server.hosted
    assert "has our package's identity.ca_fingerprint" in out
    assert Server.hosted["xaa/package"] == ours.read_bytes()
    rc, out = run("--url", server, "--ours", ours, "--key", KEY["xaa"])
    assert rc == 0 and "FED_CA_CERT is not set" in out


@needs_ed25519
@needs_openssl
def test_their_ca_is_fetched_checked_placed_and_read_back(server, tmp_path):
    ca, fp = mint_ca(tmp_path, "xab")
    (tmp_path / "xab-ca.pem").write_bytes(ca)
    theirs_pkg = package_with_ca(tmp_path, "xab", fp)
    run("--url", server, "--ours", theirs_pkg, "--key", KEY["xab"], "--ca", tmp_path / "xab-ca.pem")
    work = tmp_path / "work"
    work.mkdir()
    rc, out = run("--url", server, "--ours", XAA, "--their-id", "xab", "--theirs", work / "theirs.canonical.json",
                  "--work", work)
    assert rc == 0, out
    assert "PASS    package digest" in out and "PASS    CA fingerprint" in out
    assert (work / "theirs.canonical.json").read_bytes() == theirs_pkg.read_bytes()
    assert d.pem_certificates((work / "theirs-ca.pem").read_bytes()) == d.pem_certificates(ca)
    assert f"their verifying key (as the directory has it; confirm it, then record it in the keyring): " \
           f"{d.key_groups(PUB['xab'])}" in out
    assert f"their CA fingerprint:  {d.fingerprint_groups(fp)}" in out


@needs_ed25519
@needs_openssl
def test_a_ca_that_does_not_match_their_package_is_not_placed(server, tmp_path):
    _, fp = mint_ca(tmp_path, "xab")
    wrong, _ = mint_ca(tmp_path, "wrong")
    run("--url", server, "--ours", package_with_ca(tmp_path, "xab", fp), "--key", KEY["xab"])
    Server.hosted["xab/ca"] = wrong
    work = tmp_path / "work"
    work.mkdir()
    rc, out = run("--url", server, "--ours", XAA, "--their-id", "xab", "--work", work)
    assert rc == 0 and "FAIL    CA fingerprint" in out
    assert not (work / "theirs-ca.pem").exists()
    assert "their CA fingerprint" not in out


@needs_ed25519
def test_a_party_hosting_no_ca_is_a_warning(server, tmp_path):
    serve("xab", XAB)
    run("--url", server, "--ours", XAB, "--package-url", f"{server}/packages/xab.json", "--key", KEY["xab"])
    rc, out = run("--url", server, "--ours", XAA, "--their-id", "xab", "--work", tmp_path)
    assert rc == 0 and "WARN    CA fetch" in out and not (tmp_path / "theirs-ca.pem").exists()


@needs_ed25519
@needs_openssl
def test_driver_phase_5_without_a_package_url_uploads_and_fetches(server, tmp_path):
    ca, fp = mint_ca(tmp_path, "xab")
    (tmp_path / "xab-ca.pem").write_bytes(ca)
    run("--url", server, "--ours", package_with_ca(tmp_path, "xab", fp), "--key", KEY["xab"], "--ca", tmp_path / "xab-ca.pem")
    work = tmp_path / "work"
    work.mkdir()
    ours_ca, ours_fp = mint_ca(tmp_path, "xaa")
    (tmp_path / "xaa-ca.pem").write_bytes(ours_ca)
    (work / "ours.canonical.json").write_bytes(package_with_ca(tmp_path, "xaa", ours_fp).read_bytes())
    r = driver(work, FED_DIRECTORY_URL=server, FED_COUNTERPARTY_ID="xab", FED_SIGNING_KEY=KEY["xaa"],
               FED_CA_CERT=tmp_path / "xaa-ca.pem")
    assert r.returncode == 0, r.stderr
    assert Server.hosted["xaa/package"] == (work / "ours.canonical.json").read_bytes()
    assert "xaa/ca" in Server.hosted
    assert (work / "theirs-ca.pem").exists() and (work / "theirs.canonical.json").exists()
    assert r.stdout.rstrip().splitlines()[-1].strip().startswith("^ OPERATOR STEP")


BUNDLE = b'{"keys":[{"use":"jwt-svid","kty":"OKP","crv":"Ed25519","kid":"k1","x":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}],"spiffe_sequence":1}\n'


@needs_ed25519
@needs_openssl
def test_their_spiffe_bundle_is_uploaded_checked_against_their_key_and_placed(server, tmp_path):
    ca, fp = mint_ca(tmp_path, "xab")
    (tmp_path / "xab-ca.pem").write_bytes(ca)
    theirs_pkg = package_with_ca(tmp_path, "xab", fp)
    theirs_work = tmp_path / "theirs-work"
    theirs_work.mkdir()
    (theirs_work / "ours.spiffe-bundle").write_bytes(BUNDLE)
    _, out = run("--url", server, "--ours", theirs_pkg, "--key", KEY["xab"], "--ca", tmp_path / "xab-ca.pem",
                 "--work", theirs_work)
    assert Server.hosted["xab/spiffe-bundle"] == BUNDLE and "uploaded our SPIFFE trust bundle" in out
    work = tmp_path / "work"
    work.mkdir()
    rc, out = run("--url", server, "--ours", XAA, "--their-id", "xab", "--theirs", work / "theirs.canonical.json",
                  "--work", work)
    assert rc == 0, out
    assert "PASS    SPIFFE bundle signature" in out
    assert (work / "theirs.spiffe-bundle").read_bytes() == BUNDLE


@needs_ed25519
@needs_openssl
def test_a_spiffe_bundle_not_signed_by_their_key_is_not_placed(server, tmp_path):
    ca, fp = mint_ca(tmp_path, "xab")
    (tmp_path / "xab-ca.pem").write_bytes(ca)
    theirs_pkg = package_with_ca(tmp_path, "xab", fp)
    theirs_work = tmp_path / "theirs-work"
    theirs_work.mkdir()
    (theirs_work / "ours.spiffe-bundle").write_bytes(BUNDLE)
    run("--url", server, "--ours", theirs_pkg, "--key", KEY["xab"], "--ca", tmp_path / "xab-ca.pem", "--work", theirs_work)
    Server.hosted["xab/spiffe-bundle"] = BUNDLE.replace(b'"spiffe_sequence":1', b'"spiffe_sequence":2')
    work = tmp_path / "work"
    work.mkdir()
    rc, out = run("--url", server, "--ours", XAA, "--their-id", "xab", "--theirs", work / "theirs.canonical.json",
                  "--work", work)
    assert rc == 0, out
    assert "FAIL    SPIFFE bundle signature" in out
    assert not (work / "theirs.spiffe-bundle").exists()


# ---------------------------------------------------------------------------
# Several peerings: one package per counterparty (--per-counterparty).
# ---------------------------------------------------------------------------

def peering_package(tmp_path, sid, peer, endpoint, subnet):
    """`sid`'s package for its peering with `peer`: the compose-full package with this
    peering's own endpoint and link subnet, re-signed under the test key."""
    base = XAA if sid == "xaa" else XAB
    pkg = {k: v for k, v in json.loads(base.read_bytes()).items() if k != "signature"}
    pkg["transport_offer"] = dict(pkg["transport_offer"], endpoint=endpoint, proposed_link_subnet=subnet)
    seed = d.load_seed(str(KEY[sid]))
    raw = d.canonical(dict(pkg, signature=d.b64(d.sign(seed, d.canonical(pkg)))))
    p = tmp_path / f"{sid}-for-{peer}.json"
    p.write_bytes(raw)
    return p


def keyring_for(tmp_path, *sids):
    k = tmp_path / "keys.txt"
    k.write_text("".join(f"{s}={PUB[s]}\n" for s in sids))
    return k


@needs_ed25519
def test_a_party_with_two_counterparties_holds_two_packages_and_neither_replaces_the_other(server, tmp_path):
    for_xab = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51821", "169.254.10.0/31")
    for_xac = peering_package(tmp_path, "xaa", "xac", "192.0.2.10:51822", "169.254.10.2/31")
    rc, out = run("--url", server, "--ours", for_xab, "--key", KEY["xaa"], "--their-id", "xab", "--per-counterparty")
    assert rc == 0 and "refused" not in out, out
    rc, out = run("--url", server, "--ours", for_xac, "--key", KEY["xaa"], "--their-id", "xac", "--per-counterparty")
    assert rc == 0 and "refused" not in out, out
    assert "the single package slot holds another peering's package" in out

    assert Server.hosted["xaa/packages/xab"] == for_xab.read_bytes()
    assert Server.hosted["xaa/packages/xac"] == for_xac.read_bytes()
    # The single slot and the pointer stay with the first peering.
    assert Server.hosted["xaa/package"] == for_xab.read_bytes()
    assert json.loads(Server.entries["xaa"][0])["package_sha256"] == d.sha256_of(for_xab.read_bytes())

    # Re-running the first peering does not move anything either.
    rc, out = run("--url", server, "--ours", for_xab, "--key", KEY["xaa"], "--their-id", "xab", "--per-counterparty")
    assert rc == 0 and "published `xaa`" in out
    assert Server.hosted["xaa/packages/xac"] == for_xac.read_bytes()

    # Each counterparty reads the package meant for it, checked under the key read back.
    keyring = keyring_for(tmp_path, "xaa")
    for peer, want in (("xab", for_xab), ("xac", for_xac)):
        ours = peering_package(tmp_path, "xab", "xaa", "198.51.100.20:51821", "169.254.20.0/31")
        if peer == "xac":
            # xac has no test key of its own; it only reads, so any package of its own will do.
            pkg = json.loads(ours.read_bytes())
            pkg["sovereign_id"] = "xac"
            ours = tmp_path / "xac-ours.json"
            ours.write_bytes(d.canonical(pkg))
        theirs = tmp_path / f"{peer}-theirs.json"
        rc, out = run("--url", server, "--ours", ours, "--their-id", "xaa", "--theirs", theirs,
                      "--keyring", keyring, "--per-counterparty")
        assert rc == 0, out
        assert "PASS    slot signature" in out and "PASS    package signature" in out, out
        assert theirs.read_bytes() == want.read_bytes(), peer


@needs_ed25519
def test_a_slot_not_uploaded_under_the_bound_key_is_not_used_and_not_replaced_by_the_pointer(server, tmp_path):
    for_xab = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51821", "169.254.10.0/31")
    run("--url", server, "--ours", for_xab, "--key", KEY["xaa"], "--their-id", "xab", "--per-counterparty")
    # The directory serves the slot as if written for someone else.
    Server.signed["xaa/packages/xab"] = (PUB["xaa"], Server.signed["xaa/package"][1])
    theirs = tmp_path / "theirs.json"
    rc, out = run("--url", server, "--ours", XAB, "--their-id", "xaa", "--theirs", theirs,
                  "--keyring", keyring_for(tmp_path, "xaa"), "--per-counterparty")
    assert rc == 0 and "FAIL    slot signature" in out, out
    assert not theirs.exists() and "COMPATIBILITY REPORT" not in out


@needs_ed25519
def test_without_a_slot_for_us_the_pointers_package_is_saved_not_placed_until_confirmed(server, tmp_path, monkeypatch):
    """Their pointer may name the package of another of their peerings, so with the flag on a
    package reached through it is saved for the operator and placed only on confirmation."""
    monkeypatch.delenv("FED_DIRECTORY_PLACE_POINTER_PACKAGE", raising=False)
    serve("xab", XAB)
    run("--url", server, "--ours", XAB, "--package-url", f"{server}/packages/xab.json", "--key", KEY["xab"])
    theirs = tmp_path / "theirs.json"
    args = ("--url", server, "--ours", XAA, "--their-id", "xab", "--theirs", theirs,
            "--keyring", keyring_for(tmp_path, "xab"), "--work", tmp_path, "--per-counterparty")
    rc, out = run(*args)
    assert rc == 0, out
    assert "UNKNOWN package for us" in out and "PASS    package digest" in out
    assert not theirs.exists(), out
    assert (tmp_path / "theirs.pointer.json").read_bytes() == XAB.read_bytes()
    assert "it may belong to another of `xab`'s peerings" in out and "FED_DIRECTORY_PLACE_POINTER_PACKAGE=1" in out
    rc, out = run(*args, "--place-pointer-package")
    assert rc == 0 and theirs.read_bytes() == XAB.read_bytes(), out
    theirs.unlink()
    monkeypatch.setenv("FED_DIRECTORY_PLACE_POINTER_PACKAGE", "yes")
    rc, out = run(*args)
    assert rc == 0 and theirs.read_bytes() == XAB.read_bytes(), out


@needs_ed25519
def test_an_unreadable_place_pointer_setting_does_nothing(server, tmp_path, monkeypatch):
    monkeypatch.setenv("FED_DIRECTORY_PLACE_POINTER_PACKAGE", "maybe")
    rc, out = run("--url", server, "--ours", XAA, "--key", KEY["xaa"], "--their-id", "xab", "--per-counterparty")
    assert rc == 0 and "FAIL    FED_DIRECTORY_PLACE_POINTER_PACKAGE='maybe' is not one of" in out, out
    assert Server.entries == {} and Server.hosted == {}


def test_env_flag_reads_on_and_off_and_refuses_anything_else(monkeypatch):
    for v in ("1", "true", "YES", " yes "):
        monkeypatch.setenv("FED_X", v)
        assert d.env_flag("FED_X") is True
    for v in ("", "0", "false", "No"):
        monkeypatch.setenv("FED_X", v)
        assert d.env_flag("FED_X") is False
    monkeypatch.delenv("FED_X")
    assert d.env_flag("FED_X") is False
    for v in ("2", "on", "y", "truthy"):
        monkeypatch.setenv("FED_X", v)
        with pytest.raises(ValueError, match="FED_X"):
            d.env_flag("FED_X")


@needs_ed25519
def test_without_the_flag_the_pointers_package_is_still_placed(server, tmp_path, monkeypatch):
    monkeypatch.delenv("FED_DIRECTORY_PER_COUNTERPARTY", raising=False)
    serve("xab", XAB)
    run("--url", server, "--ours", XAB, "--package-url", f"{server}/packages/xab.json", "--key", KEY["xab"])
    theirs = tmp_path / "theirs.json"
    rc, out = run("--url", server, "--ours", XAA, "--their-id", "xab", "--theirs", theirs, "--work", tmp_path)
    assert rc == 0 and theirs.read_bytes() == XAB.read_bytes(), out
    assert not (tmp_path / "theirs.pointer.json").exists()


@needs_ed25519
def test_an_older_directory_without_slots_keeps_the_single_package_flow(server, tmp_path):
    Server.no_slots = True
    rc, out = run("--url", server, "--ours", XAA, "--key", KEY["xaa"], "--their-id", "xab", "--per-counterparty")
    assert rc == 0
    assert "does not host a package per counterparty (HTTP 404" in out
    assert Server.hosted["xaa/package"] == XAA.read_bytes()


@needs_ed25519
def test_without_the_flag_the_single_package_flow_is_unchanged(server, tmp_path, monkeypatch):
    monkeypatch.delenv("FED_DIRECTORY_PER_COUNTERPARTY", raising=False)
    for_xab = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51821", "169.254.10.0/31")
    for_xac = peering_package(tmp_path, "xaa", "xac", "192.0.2.10:51822", "169.254.10.2/31")
    run("--url", server, "--ours", for_xab, "--key", KEY["xaa"], "--their-id", "xab")
    assert sorted(Server.hosted) == ["xaa/package"]
    assert Server.hosted["xaa/package"] == for_xab.read_bytes()
    # Today's behavior, kept as the default: the second peering's package replaces the first.
    run("--url", server, "--ours", for_xac, "--key", KEY["xaa"], "--their-id", "xac")
    assert sorted(Server.hosted) == ["xaa/package"]
    assert Server.hosted["xaa/package"] == for_xac.read_bytes()
    assert not any("/packages/" in r for r in Server.hosted)


@needs_ed25519
def test_the_flag_also_comes_from_the_environment(server, tmp_path, monkeypatch):
    monkeypatch.setenv("FED_DIRECTORY_PER_COUNTERPARTY", "1")
    rc, out = run("--url", server, "--ours", XAA, "--key", KEY["xaa"], "--their-id", "xab")
    assert rc == 0 and Server.hosted["xaa/packages/xab"] == XAA.read_bytes(), out


@pytest.fixture(scope="session")
def fdverify(tmp_path_factory):
    """The reference implementation's composer (ops/federation/reference), independent of the Rust one."""
    if not shutil.which("go"):
        pytest.skip("no Go toolchain to build ops/federation/reference")
    exe = tmp_path_factory.mktemp("fdverify") / "fdverify"
    subprocess.run(["go", "build", "-o", str(exe), "./cmd/fdverify"], cwd=REPO / "ops/federation/reference",
                   check=True, capture_output=True)
    return exe


@needs_ed25519
def test_an_existing_agreement_keeps_its_bytes_and_digest_through_counterparty_slots(server, tmp_path, fdverify):
    """The compose-full vector pair, exchanged through per-counterparty slots, composes to the vector's
    agreement bytes and digest: the slot moves where a package lives, never its bytes."""
    vec = VEC / "compose-full"
    now = (VEC / "now.txt").read_text().strip()
    run("--url", server, "--ours", XAA, "--key", KEY["xaa"], "--their-id", "xab", "--per-counterparty")
    run("--url", server, "--ours", XAB, "--key", KEY["xab"], "--their-id", "xaa", "--per-counterparty")
    got = {}
    for us, them in (("xaa", "xab"), ("xab", "xaa")):
        theirs = tmp_path / f"{us}-theirs.json"
        rc, out = run("--url", server, "--ours", XAA if us == "xaa" else XAB, "--their-id", them,
                      "--theirs", theirs, "--keyring", keyring_for(tmp_path, them), "--per-counterparty")
        assert rc == 0 and "PASS    slot signature" in out, out
        got[them] = theirs
    assert got["xab"].read_bytes() == XAB.read_bytes() and got["xaa"].read_bytes() == XAA.read_bytes()
    r = subprocess.run([str(fdverify), "compose", "-now", now, "-keydir", str(VEC / "testkeys"),
                        str(got["xaa"]), str(got["xab"])], capture_output=True, check=True)
    assert r.stdout == (vec / "agreement.canonical.json").read_bytes()
    r = subprocess.run([str(fdverify), "compose", "-now", now, "-digest", "-keydir", str(VEC / "testkeys"),
                        str(got["xab"]), str(got["xaa"])], capture_output=True, check=True, text=True)
    # Composed in the other argument order, so position comes from sovereign_id, not from who ran it.
    assert r.stdout.encode() == (vec / "agreement.canonical.json").read_bytes()
    assert (vec / "agreement.digest.txt").read_text().strip() in r.stderr


@needs_ed25519
def test_a_failed_slot_read_does_not_fall_back_to_the_pointer(server, tmp_path):
    """Their pointer may name the package of another of their peerings, signed by the same key,
    so only a 404 sends the lookup to it."""
    for_xab = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51821", "169.254.10.0/31")
    for_xac = peering_package(tmp_path, "xaa", "xac", "192.0.2.10:51822", "169.254.10.2/31")
    run("--url", server, "--ours", for_xac, "--key", KEY["xaa"], "--their-id", "xac", "--per-counterparty")
    run("--url", server, "--ours", for_xab, "--key", KEY["xaa"], "--their-id", "xab", "--per-counterparty")
    assert Server.hosted["xaa/package"] == for_xac.read_bytes()  # the pointer serves the xac peering
    Server.fail_slot_reads = 503
    theirs = tmp_path / "theirs.json"
    rc, out = run("--url", server, "--ours", XAB, "--their-id", "xaa", "--theirs", theirs,
                  "--keyring", keyring_for(tmp_path, "xaa"), "--per-counterparty")
    assert rc == 0 and "their pointer is not read in its place" in out, out
    assert "package digest" not in out and not theirs.exists()


@needs_ed25519
def test_a_slot_written_under_another_key_or_with_a_broken_keyring_key_fails(server, tmp_path):
    run("--url", server, "--ours", XAA, "--key", KEY["xaa"], "--their-id", "xab", "--per-counterparty")
    theirs = tmp_path / "theirs.json"
    Server.signed["xaa/packages/xab"] = (PUB["xab"], Server.signed["xaa/packages/xab"][1])
    rc, out = run("--url", server, "--ours", XAB, "--their-id", "xaa", "--theirs", theirs,
                  "--keyring", keyring_for(tmp_path, "xaa"), "--per-counterparty")
    assert "FAIL    slot signature" in out and not theirs.exists(), out
    bad = tmp_path / "bad-keys.txt"
    bad.write_text("xaa=not-a-key\n")
    rc, out = run("--url", server, "--ours", XAB, "--their-id", "xaa", "--theirs", theirs,
                  "--keyring", bad, "--per-counterparty")
    assert rc == 0 and "is not 32 bytes of base64" in out and not theirs.exists(), out


@needs_ed25519
def test_a_reissue_of_the_first_peering_with_a_changed_offer_still_refreshes_the_single_slot(server, tmp_path):
    work_ab, work_ac = tmp_path / "ab", tmp_path / "ac"
    work_ab.mkdir()
    work_ac.mkdir()
    first = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51821", "169.254.10.0/31")
    other = peering_package(tmp_path, "xaa", "xac", "192.0.2.10:51822", "169.254.10.2/31")
    run("--url", server, "--ours", first, "--key", KEY["xaa"], "--their-id", "xab", "--work", work_ab, "--per-counterparty")
    run("--url", server, "--ours", other, "--key", KEY["xaa"], "--their-id", "xac", "--work", work_ac, "--per-counterparty")
    assert Server.hosted["xaa/package"] == first.read_bytes()
    # The xab peering moves its port: a different transport_offer, the same peering.
    moved = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51831", "169.254.10.0/31")
    rc, out = run("--url", server, "--ours", moved, "--key", KEY["xaa"], "--their-id", "xab", "--work", work_ab,
                  "--per-counterparty")
    assert rc == 0, out
    assert Server.hosted["xaa/package"] == moved.read_bytes()
    assert json.loads(Server.entries["xaa"][0])["package_sha256"] == d.sha256_of(moved.read_bytes())
    assert Server.hosted["xaa/packages/xab"] == moved.read_bytes()
    assert Server.hosted["xaa/packages/xac"] == other.read_bytes()


@needs_ed25519
def test_with_our_own_host_the_pointer_stays_with_the_peering_it_names(server, tmp_path):
    for_xab = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51821", "169.254.10.0/31")
    for_xac = peering_package(tmp_path, "xaa", "xac", "192.0.2.10:51822", "169.254.10.2/31")
    Server.files["/packages/xaa-xab.json"] = for_xab.read_bytes()
    Server.files["/packages/xaa-xac.json"] = for_xac.read_bytes()
    run("--url", server, "--ours", for_xab, "--key", KEY["xaa"], "--package-url", f"{server}/packages/xaa-xab.json",
        "--their-id", "xab", "--per-counterparty")
    rc, out = run("--url", server, "--ours", for_xac, "--key", KEY["xaa"], "--package-url",
                  f"{server}/packages/xaa-xac.json", "--their-id", "xac", "--per-counterparty")
    assert rc == 0 and "our pointer names another peering's package" in out, out
    pointer = json.loads(Server.entries["xaa"][0])
    assert pointer["package_url"] == f"{server}/packages/xaa-xab.json"
    assert Server.hosted["xaa/packages/xac"] == for_xac.read_bytes()
    assert "xaa/package" not in Server.hosted


@needs_ed25519
def test_a_slot_served_with_another_slots_upload_is_not_placed_even_without_a_keyring(server, tmp_path):
    """The slot's resource line is the check that the package is for us, so it is verified
    before placing even when no key has been read back yet (under the directory's bound key)."""
    for_xab = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51821", "169.254.10.0/31")
    for_xac = peering_package(tmp_path, "xaa", "xac", "192.0.2.10:51822", "169.254.10.2/31")
    run("--url", server, "--ours", for_xab, "--key", KEY["xaa"], "--their-id", "xab", "--per-counterparty")
    run("--url", server, "--ours", for_xac, "--key", KEY["xaa"], "--their-id", "xac", "--per-counterparty")
    # The directory serves xac's package, with xac's genuine upload signature, in xab's slot.
    Server.hosted["xaa/packages/xab"] = Server.hosted["xaa/packages/xac"]
    Server.signed["xaa/packages/xab"] = Server.signed["xaa/packages/xac"]
    theirs = tmp_path / "theirs.json"
    rc, out = run("--url", server, "--ours", XAB, "--their-id", "xaa", "--theirs", theirs, "--per-counterparty")
    assert rc == 0 and "FAIL    slot signature" in out, out
    assert not theirs.exists() and "COMPATIBILITY REPORT" not in out
    # A slot served with no upload signature at all is not placed either.
    Server.hosted["xaa/packages/xab"] = for_xab.read_bytes()
    del Server.signed["xaa/packages/xab"]
    rc, out = run("--url", server, "--ours", XAB, "--their-id", "xaa", "--theirs", theirs, "--per-counterparty")
    assert rc == 0 and "FAIL    slot signature" in out and not theirs.exists(), out


@needs_ed25519
def test_without_a_keyring_a_slot_whose_resource_names_us_is_placed_and_its_key_printed(server, tmp_path):
    for_xab = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51821", "169.254.10.0/31")
    run("--url", server, "--ours", for_xab, "--key", KEY["xaa"], "--their-id", "xab", "--per-counterparty")
    theirs = tmp_path / "theirs.json"
    rc, out = run("--url", server, "--ours", XAB, "--their-id", "xaa", "--theirs", theirs, "--per-counterparty")
    assert rc == 0 and "PASS    slot resource" in out and "UNKNOWN slot signature" in out, out
    assert theirs.read_bytes() == for_xab.read_bytes()
    assert f"their verifying key (as the directory has it; confirm it, then record it in the keyring): " \
           f"{d.key_groups(PUB['xaa'])}" in out


@needs_ed25519
def test_a_party_with_one_peering_from_the_prior_release_keeps_refreshing_its_single_slot(server, tmp_path, monkeypatch):
    """The prior release wrote ours.pointer.json but no mark file. A reissue with a changed offer
    under the flag must still count the single slot as this peering's."""
    monkeypatch.delenv("FED_DIRECTORY_PER_COUNTERPARTY", raising=False)
    first = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51821", "169.254.10.0/31")
    rc, out = run("--url", server, "--ours", first, "--key", KEY["xaa"], "--their-id", "xab", "--work", tmp_path)
    assert rc == 0 and Server.hosted["xaa/package"] == first.read_bytes(), out
    # Without the flag the step writes no mark, as the prior release did not.
    assert not (tmp_path / d.SINGLE_SLOT_MARK).exists()
    moved = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51831", "169.254.10.0/31")
    rc, out = run("--url", server, "--ours", moved, "--key", KEY["xaa"], "--their-id", "xab", "--work", tmp_path,
                  "--per-counterparty")
    assert rc == 0, out
    assert "another peering's package" not in out
    assert Server.hosted["xaa/package"] == moved.read_bytes()
    assert json.loads(Server.entries["xaa"][0])["package_sha256"] == d.sha256_of(moved.read_bytes())
    assert Server.hosted["xaa/packages/xab"] == moved.read_bytes()


@needs_ed25519
def test_with_the_flag_and_no_counterparty_id_a_second_peering_replaces_nothing(server, tmp_path):
    for_xab = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51821", "169.254.10.0/31")
    for_xac = peering_package(tmp_path, "xaa", "xac", "192.0.2.10:51822", "169.254.10.2/31")
    run("--url", server, "--ours", for_xab, "--key", KEY["xaa"], "--their-id", "xab", "--per-counterparty")
    rc, out = run("--url", server, "--ours", for_xac, "--key", KEY["xaa"], "--per-counterparty")
    assert rc == 0, out
    assert "no per-counterparty slot was published" in out
    assert Server.hosted["xaa/package"] == for_xab.read_bytes()
    assert json.loads(Server.entries["xaa"][0])["package_sha256"] == d.sha256_of(for_xab.read_bytes())
    assert sorted(Server.hosted) == ["xaa/package", "xaa/packages/xab"]


@needs_ed25519
def test_the_digest_does_not_tell_apart_packages_that_differ_only_in_members_the_agreement_omits(tmp_path, fdverify):
    """Why the slot signature, not the digest, is the misdelivery check (spec §11.1): xab's package
    re-signed with another endpoint, WireGuard key and link subnet composes with xaa's to the same
    agreement bytes and digest, because the agreement carries neither endpoint nor public_key and
    takes link_subnet from parties[0] (xaa)."""
    vec = VEC / "compose-full"
    now = (VEC / "now.txt").read_text().strip()
    pkg = {k: v for k, v in json.loads(XAB.read_bytes()).items() if k != "signature"}
    offer = dict(pkg["transport_offer"])
    assert {"endpoint", "public_key", "proposed_link_subnet"} <= set(offer)
    offer.update(endpoint="203.0.113.99:51999", proposed_link_subnet="169.254.99.0/31",
                 public_key=base64.b64encode(bytes(range(32))).decode())
    assert offer != pkg["transport_offer"]
    pkg["transport_offer"] = offer
    seed = d.load_seed(str(KEY["xab"]))
    other = tmp_path / "xab-for-someone-else.json"
    other.write_bytes(d.canonical(dict(pkg, signature=d.b64(d.sign(seed, d.canonical(pkg))))))
    assert other.read_bytes() != XAB.read_bytes()
    r = subprocess.run([str(fdverify), "compose", "-now", now, "-digest", "-keydir", str(VEC / "testkeys"),
                        str(XAA), str(other)], capture_output=True, check=True, text=True)
    assert r.stdout.encode() == (vec / "agreement.canonical.json").read_bytes()
    assert (vec / "agreement.digest.txt").read_text().strip() in r.stderr


ROGUE_SEED = bytes(range(1, 33))  # a key the directory holds, not any party's


def rogue_slot(resource, body):
    """Make the stand-in serve body at resource with an upload statement the directory signed
    under a key of its own, and that key as the party's bound key: a substituted key."""
    key = d.b64(d.public_key(ROGUE_SEED))
    Server.hosted[resource] = body
    Server.signed[resource] = (key, d.b64(d.sign(ROGUE_SEED, d.request_statement("PUT", resource, body, key))))
    Server.bound_override[resource.split("/")[0]] = key
    return key


@needs_ed25519
def test_a_directory_that_substitutes_its_own_key_cannot_carry_a_genuine_package(server, tmp_path):
    """Without a keyring, a directory signing the slot statement under a key it holds and serving
    another peering's genuine package (signed by the party's real key) is refused: the package's
    own signature must verify under the same key as the upload."""
    for_xab = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51821", "169.254.10.0/31")
    for_xac = peering_package(tmp_path, "xaa", "xac", "192.0.2.10:51822", "169.254.10.2/31")
    run("--url", server, "--ours", for_xab, "--key", KEY["xaa"], "--their-id", "xab", "--per-counterparty")
    rogue_slot("xaa/packages/xab", for_xac.read_bytes())
    theirs = tmp_path / "theirs.json"
    rc, out = run("--url", server, "--ours", XAB, "--their-id", "xaa", "--theirs", theirs, "--per-counterparty")
    assert rc == 0 and "FAIL    package signature" in out, out
    assert not theirs.exists() and "COMPATIBILITY REPORT" not in out


@needs_ed25519
def test_a_package_placed_before_the_key_was_read_back_is_moved_aside_when_it_then_fails(server, tmp_path):
    """A directory consistent with a key of its own gets its package placed before the call; once
    the key read back is in the keyring, the slot fails and the placed file is moved aside."""
    run("--url", server, "--ours", XAA, "--key", KEY["xaa"], "--their-id", "xab", "--per-counterparty")
    pkg = {k: v for k, v in json.loads(XAA.read_bytes()).items() if k != "signature"}
    forged = d.canonical(dict(pkg, signature=d.b64(d.sign(ROGUE_SEED, d.canonical(pkg)))))
    rogue_slot("xaa/packages/xab", forged)
    theirs = tmp_path / "theirs.json"
    rc, out = run("--url", server, "--ours", XAB, "--their-id", "xaa", "--theirs", theirs, "--per-counterparty")
    assert rc == 0 and theirs.read_bytes() == forged, out  # nothing before the call can tell
    rc, out = run("--url", server, "--ours", XAB, "--their-id", "xaa", "--theirs", theirs,
                  "--keyring", keyring_for(tmp_path, "xaa"), "--per-counterparty")
    assert rc == 0 and "FAIL    slot signature" in out, out
    assert not theirs.exists() and refused(theirs).read_bytes() == forged
    assert "moved it to" in out


@needs_ed25519
def test_a_transient_failure_does_not_strand_a_prior_release_party(server, tmp_path, monkeypatch):
    """ours.pointer.json is the record of the package this peering last published; a run that
    stops before publishing must not overwrite it, or the next run no longer recognizes the slot."""
    monkeypatch.delenv("FED_DIRECTORY_PER_COUNTERPARTY", raising=False)
    first = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51821", "169.254.10.0/31")
    run("--url", server, "--ours", first, "--key", KEY["xaa"], "--their-id", "xab", "--work", tmp_path)
    assert not (tmp_path / d.SINGLE_SLOT_MARK).exists()  # without the flag, as the prior release: no mark
    first = first.read_bytes()  # peering_package reuses the file name for the same peering
    moved = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51831", "169.254.10.0/31")
    args = ("--url", server, "--ours", moved, "--key", KEY["xaa"], "--their-id", "xab", "--work", tmp_path,
            "--per-counterparty")
    Server.fail_reads["xaa/package"] = 1
    rc, out = run(*args)
    assert rc == 0 and "nothing published" in out and Server.hosted["xaa/package"] == first, out
    assert json.loads((tmp_path / "ours.pointer.json").read_bytes())["package_sha256"] == d.sha256_of(first)
    rc, out = run(*args)
    assert rc == 0 and Server.hosted["xaa/package"] == moved.read_bytes(), out


@needs_ed25519
def test_the_per_counterparty_variable_reads_true_and_yes_and_refuses_other_words(server, tmp_path, monkeypatch):
    for v in ("true", "yes", "1"):
        Server.hosted.clear()
        monkeypatch.setenv("FED_DIRECTORY_PER_COUNTERPARTY", v)
        rc, out = run("--url", server, "--ours", XAA, "--key", KEY["xaa"], "--their-id", "xab")
        assert rc == 0 and "xaa/packages/xab" in Server.hosted, (v, out)
    for v in ("false", "no", "0", ""):
        Server.hosted.clear()
        monkeypatch.setenv("FED_DIRECTORY_PER_COUNTERPARTY", v)
        rc, out = run("--url", server, "--ours", XAA, "--key", KEY["xaa"], "--their-id", "xab")
        assert rc == 0 and sorted(Server.hosted) == ["xaa/package"], (v, out)
    Server.entries.clear()
    Server.hosted.clear()
    monkeypatch.setenv("FED_DIRECTORY_PER_COUNTERPARTY", "on")
    rc, out = run("--url", server, "--ours", XAA, "--key", KEY["xaa"], "--their-id", "xab")
    assert rc == 0 and "FAIL    FED_DIRECTORY_PER_COUNTERPARTY='on'" in out and Server.hosted == {}, out


@needs_ed25519
@needs_openssl
def test_a_held_package_places_no_ca_or_spiffe_bundle_either(server, tmp_path, monkeypatch):
    monkeypatch.delenv("FED_DIRECTORY_PLACE_POINTER_PACKAGE", raising=False)
    ca, fp = mint_ca(tmp_path, "xab")
    (tmp_path / "xab-ca.pem").write_bytes(ca)
    theirs_work = tmp_path / "theirs-work"
    theirs_work.mkdir()
    (theirs_work / "ours.spiffe-bundle").write_bytes(BUNDLE)
    run("--url", server, "--ours", package_with_ca(tmp_path, "xab", fp), "--key", KEY["xab"],
        "--ca", tmp_path / "xab-ca.pem", "--work", theirs_work)
    work = tmp_path / "work"
    work.mkdir()
    rc, out = run("--url", server, "--ours", XAA, "--their-id", "xab", "--theirs", work / "theirs.canonical.json",
                  "--work", work, "--per-counterparty")
    assert rc == 0 and (work / "theirs.pointer.json").exists(), out
    assert not (work / "theirs.canonical.json").exists()
    assert not (work / "theirs-ca.pem").exists() and not (work / "theirs.spiffe-bundle").exists(), out
    assert "UNKNOWN CA fetch" in out


def publish_with_ca_and_bundle(server, tmp_path, sid="xab", their_id="xaa"):
    """`sid` publishes its package (with a slot for their_id), its CA and its SPIFFE bundle."""
    ca, fp = mint_ca(tmp_path, sid)
    (tmp_path / f"{sid}-ca.pem").write_bytes(ca)
    pkg = package_with_ca(tmp_path, sid, fp)
    w = tmp_path / f"{sid}-own-work"
    w.mkdir()
    (w / "ours.spiffe-bundle").write_bytes(BUNDLE)
    run("--url", server, "--ours", pkg, "--key", KEY[sid], "--ca", tmp_path / f"{sid}-ca.pem", "--work", w,
        "--their-id", their_id, "--per-counterparty")
    return pkg, ca


@needs_ed25519
@needs_openssl
def test_before_the_key_is_recorded_the_ca_and_bundle_are_saved_unconfirmed_never_in_place(server, tmp_path):
    pkg, ca = publish_with_ca_and_bundle(server, tmp_path)
    work = tmp_path / "work"
    work.mkdir()
    args = ("--url", server, "--ours", XAA, "--their-id", "xab", "--theirs", work / "theirs.canonical.json",
            "--work", work, "--per-counterparty")
    rc, out = run(*args)
    assert rc == 0, out
    assert not (work / "theirs-ca.pem").exists() and not (work / "theirs.spiffe-bundle").exists(), out
    assert d.pem_certificates((work / "theirs-ca.pem.unconfirmed").read_bytes()) == d.pem_certificates(ca)
    assert (work / "theirs.spiffe-bundle.unconfirmed").read_bytes() == BUNDLE
    # With the key recorded, the next run places both and drops the unconfirmed copies.
    rc, out = run(*args, "--keyring", keyring_for(tmp_path, "xab"))
    assert rc == 0, out
    assert d.pem_certificates((work / "theirs-ca.pem").read_bytes()) == d.pem_certificates(ca)
    assert (work / "theirs.spiffe-bundle").read_bytes() == BUNDLE
    assert not (work / "theirs-ca.pem.unconfirmed").exists() and not (work / "theirs.spiffe-bundle.unconfirmed").exists()


@needs_ed25519
@needs_openssl
def test_a_refused_package_takes_its_ca_and_bundle_aside_with_it(server, tmp_path):
    """Files an earlier run placed from a package that does not verify under the recorded key
    are all moved aside: the package, and the CA and bundle it vouched for."""
    publish_with_ca_and_bundle(server, tmp_path)
    work = tmp_path / "work"
    work.mkdir()
    pkg = {k: v for k, v in json.loads(XAB.read_bytes()).items() if k != "signature"}
    forged = d.canonical(dict(pkg, signature=d.b64(d.sign(ROGUE_SEED, d.canonical(pkg)))))
    (work / "theirs.canonical.json").write_bytes(forged)
    (work / "theirs-ca.pem").write_bytes(b"rogue CA")
    (work / "theirs.spiffe-bundle").write_bytes(b"rogue bundle")
    rc, out = run("--url", server, "--ours", XAA, "--their-id", "xab", "--theirs", work / "theirs.canonical.json",
                  "--work", work, "--keyring", keyring_for(tmp_path, "xab"), "--per-counterparty")
    assert rc == 0, out
    assert refused(work / "theirs.canonical.json").read_bytes() == forged
    assert refused(work / "theirs-ca.pem").read_bytes() == b"rogue CA"
    assert refused(work / "theirs.spiffe-bundle").read_bytes() == b"rogue bundle"
    # The directory's genuine package for us then takes their place.
    assert (work / "theirs.spiffe-bundle").read_bytes() == BUNDLE


@needs_ed25519
@needs_openssl
def test_a_replayed_older_genuine_bundle_and_ca_change_nothing_on_disk(server, tmp_path):
    """A signature carries no date, so a directory can serve any bundle or package the partner
    ever uploaded. After the partner rotates its CA and bundle and we place the new ones, the
    directory replaying the older genuine uploads must not move them aside or replace them."""
    ca1, fp1 = mint_ca(tmp_path, "xab-old")
    ca2, fp2 = mint_ca(tmp_path, "xab-new")
    bundle1, bundle2 = BUNDLE, BUNDLE.replace(b'"spiffe_sequence":1', b'"spiffe_sequence":2')
    own = tmp_path / "xab-own-work"
    own.mkdir()

    def publish(ca, fp, bundle):
        (tmp_path / "xab-ca.pem").write_bytes(ca)
        pkg = package_with_ca(tmp_path, "xab", fp).read_bytes()
        (tmp_path / "xab-pkg.json").write_bytes(pkg)
        (own / "ours.spiffe-bundle").write_bytes(bundle)
        run("--url", server, "--ours", tmp_path / "xab-pkg.json", "--key", KEY["xab"], "--ca", tmp_path / "xab-ca.pem",
            "--work", own, "--their-id", "xaa", "--per-counterparty")
        return pkg

    old_pkg = publish(ca1, fp1, bundle1)
    saved = {r: (Server.hosted[r], Server.signed.get(r)) for r in ("xab/packages/xaa", "xab/ca", "xab/spiffe-bundle")}
    new_pkg = publish(ca2, fp2, bundle2)
    work = tmp_path / "work"
    work.mkdir()
    args = ("--url", server, "--ours", XAA, "--their-id", "xab", "--theirs", work / "theirs.canonical.json",
            "--work", work, "--keyring", keyring_for(tmp_path, "xab"), "--per-counterparty")
    rc, out = run(*args)
    assert rc == 0 and (work / "theirs.canonical.json").read_bytes() == new_pkg, out
    assert (work / "theirs.spiffe-bundle").read_bytes() == bundle2
    assert d.pem_certificates((work / "theirs-ca.pem").read_bytes()) == d.pem_certificates(ca2)

    # The directory replays the older genuine uploads, each with its genuine signature.
    for r, (body, sig) in saved.items():
        Server.hosted[r] = body
        if sig:
            Server.signed[r] = sig
    rc, out = run(*args)
    assert rc == 0, out
    assert "PASS    SPIFFE bundle signature" in out  # the replayed bundle does verify
    assert (work / "theirs.canonical.json").read_bytes() == new_pkg
    assert (work / "theirs.spiffe-bundle").read_bytes() == bundle2
    assert d.pem_certificates((work / "theirs-ca.pem").read_bytes()) == d.pem_certificates(ca2)
    assert not list(work.glob("*.refused*")), out
    assert "WARN    theirs.spiffe-bundle" in out and "WARN    theirs-ca.pem" in out
    assert old_pkg != new_pkg


@needs_ed25519
def test_a_second_refusal_keeps_the_first_refused_copy(server, tmp_path):
    run("--url", server, "--ours", XAA, "--key", KEY["xaa"], "--their-id", "xab", "--per-counterparty")
    pkg = {k: v for k, v in json.loads(XAA.read_bytes()).items() if k != "signature"}
    theirs = tmp_path / "theirs.json"
    copies = []
    for n in (1, 2):
        forged_pkg = dict(pkg, expires=f"299{n}-01-01")
        forged = d.canonical(dict(forged_pkg, signature=d.b64(d.sign(ROGUE_SEED, d.canonical(forged_pkg)))))
        theirs.write_bytes(forged)
        copies.append(forged)
        rc, out = run("--url", server, "--ours", XAB, "--their-id", "xaa", "--theirs", theirs,
                      "--keyring", keyring_for(tmp_path, "xaa"), "--per-counterparty")
        assert rc == 0 and "moved it to" in out, out
        # the genuine slot package then takes its place; clear it for the next round
        theirs.unlink()
    got = sorted(p.read_bytes() for p in tmp_path.glob("theirs.json.refused-*"))
    assert got == sorted(copies)


@needs_ed25519
def test_a_lying_directory_cannot_move_a_correct_package_aside(server, tmp_path):
    """The on-disk package verifies under the recorded key; the directory serves the same bytes
    under a substituted upload key. The slot fails and nothing new is placed, but the correct
    file stays where it is."""
    for_xab = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51821", "169.254.10.0/31")
    run("--url", server, "--ours", for_xab, "--key", KEY["xaa"], "--their-id", "xab", "--per-counterparty")
    theirs = tmp_path / "theirs.json"
    theirs.write_bytes(for_xab.read_bytes())
    rogue_slot("xaa/packages/xab", for_xab.read_bytes())
    rc, out = run("--url", server, "--ours", XAB, "--their-id", "xaa", "--theirs", theirs,
                  "--keyring", keyring_for(tmp_path, "xaa"), "--per-counterparty")
    assert rc == 0 and "FAIL    slot signature" in out, out
    assert theirs.read_bytes() == for_xab.read_bytes() and not list(tmp_path.glob("theirs.json.refused*"))


@needs_ed25519
def test_a_failed_package_upload_does_not_strand_a_prior_release_party(server, tmp_path, monkeypatch):
    """Hosted: the pointer PUT can succeed and the package PUT fail. ours.pointer.json waits for the
    package to read back, so the next run still recognizes the single slot."""
    monkeypatch.delenv("FED_DIRECTORY_PER_COUNTERPARTY", raising=False)
    first = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51821", "169.254.10.0/31").read_bytes()
    first_p = tmp_path / "first.json"
    first_p.write_bytes(first)
    run("--url", server, "--ours", first_p, "--key", KEY["xaa"], "--their-id", "xab", "--work", tmp_path)
    assert not (tmp_path / d.SINGLE_SLOT_MARK).exists()  # without the flag, as the prior release: no mark
    moved = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51831", "169.254.10.0/31")
    args = ("--url", server, "--ours", moved, "--key", KEY["xaa"], "--their-id", "xab", "--work", tmp_path,
            "--per-counterparty")
    Server.fail_puts["xaa/package"] = 1
    rc, out = run(*args)
    assert rc == 0 and "package upload refused, HTTP 503" in out, out
    assert Server.hosted["xaa/package"] == first
    assert json.loads((tmp_path / "ours.pointer.json").read_bytes())["package_sha256"] == d.sha256_of(first)
    rc, out = run(*args)
    assert rc == 0 and Server.hosted["xaa/package"] == moved.read_bytes(), out


@needs_ed25519
def test_a_409_on_the_slot_names_what_the_counterparty_reads_and_the_two_remedies(server, tmp_path):
    Server.refuse_slot_with = 409
    rc, out = run("--url", server, "--ours", XAA, "--key", KEY["xaa"], "--their-id", "xab", "--per-counterparty")
    assert rc == 0, out
    assert "`xab` still reads whatever the slot holds now" in out
    assert "withdraw the slot" in out and "reissue the package with a changed field" in out


@needs_ed25519
def test_on_the_pointer_path_a_package_not_signed_by_the_recorded_key_is_not_placed(server, tmp_path, monkeypatch):
    monkeypatch.delenv("FED_DIRECTORY_PER_COUNTERPARTY", raising=False)
    serve("xab", XAB)
    run("--url", server, "--ours", XAB, "--package-url", f"{server}/packages/xab.json", "--key", KEY["xab"])
    wrong = tmp_path / "wrong-keys.txt"
    wrong.write_text(f"xab={PUB['xaa']}\n")  # a key that did not sign their package
    theirs = tmp_path / "theirs.json"
    rc, out = run("--url", server, "--ours", XAA, "--their-id", "xab", "--theirs", theirs, "--keyring", wrong,
                  "--per-counterparty", "--place-pointer-package")
    assert rc == 0 and "FAIL    package signature" in out, out
    assert not theirs.exists() and "COMPATIBILITY REPORT" not in out
    # Without the flag, as before: a WARN, and the package is placed (compose then refuses it).
    rc, out = run("--url", server, "--ours", XAA, "--their-id", "xab", "--theirs", theirs, "--keyring", wrong)
    assert rc == 0 and "WARN    package signature" in out and theirs.exists(), out


# ---------------------------------------------------------------------------
# Without the flag, the step is the single-package flow it was before slots.
# tests/fed_directory_before_slots.py is an unmodified copy of directory.py as it was on
# the integration branch before this change; each scenario below runs once through it and
# once through the current directory.py against the same stand-in, and everything either
# wrote (work directory, directory state) and printed must be identical.
# ---------------------------------------------------------------------------

_old_spec = importlib.util.spec_from_file_location("fed_directory_before_slots", REPO / "tests/fed_directory_before_slots.py")
old = importlib.util.module_from_spec(_old_spec)
_old_spec.loader.exec_module(old)


def _tree(root):
    return {str(p.relative_to(root)): p.read_bytes() for p in sorted(Path(root).rglob("*")) if p.is_file()}


def _snapshot(module, server, root, steps):
    """Run steps (a function given a runner and a fresh root) through module, from an empty
    stand-in; return what was written and printed."""
    Server.entries, Server.files, Server.hosted, Server.signed, Server.refuse_with = {}, {}, {}, {}, None
    Server.no_slots, Server.fail_slot_reads, Server.fail_reads, Server.bound_override = False, None, {}, {}
    Server.fail_puts, Server.refuse_slot_with = {}, None
    root.mkdir()
    printed = []

    def runner(*args):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = module.main([str(a) for a in args])
        printed.append((rc, out.getvalue().replace(str(root), "<root>")))

    steps(runner, root)
    state = {"hosted": dict(Server.hosted), "entries": {k: v[0] for k, v in Server.entries.items()},
             "signed": dict(Server.signed)}
    return _tree(root), state, printed


def _same_with_and_without_this_change(server, tmp_path, steps, monkeypatch):
    for var in ("FED_DIRECTORY_PER_COUNTERPARTY", "FED_DIRECTORY_PLACE_POINTER_PACKAGE"):
        monkeypatch.delenv(var, raising=False)
    before = _snapshot(old, server, tmp_path / "before", steps)
    after = _snapshot(d, server, tmp_path / "after", steps)
    assert before[0] == after[0], "work directories differ"
    assert before[1] == after[1], "directory state differs"
    assert before[2] == after[2], "printed output differs"
    return after


@needs_ed25519
@needs_openssl
def test_without_the_flag_every_file_written_and_line_printed_is_as_before(server, tmp_path, monkeypatch):
    ca_xab, fp_xab = mint_ca(tmp_path, "xab")
    ca_xaa, fp_xaa = mint_ca(tmp_path, "xaa")
    (tmp_path / "xab-ca.pem").write_bytes(ca_xab)
    (tmp_path / "xaa-ca.pem").write_bytes(ca_xaa)
    pkg_xab = package_with_ca(tmp_path, "xab", fp_xab).read_bytes()
    pkg_xaa = package_with_ca(tmp_path, "xaa", fp_xaa).read_bytes()
    (tmp_path / "pkg-xab.json").write_bytes(pkg_xab)
    (tmp_path / "pkg-xaa.json").write_bytes(pkg_xaa)
    first = peering_package(tmp_path, "xaa", "xab", "192.0.2.10:51821", "169.254.10.0/31").read_bytes()
    second = peering_package(tmp_path, "xaa", "xac", "192.0.2.10:51822", "169.254.10.2/31").read_bytes()
    (tmp_path / "first.json").write_bytes(first)
    (tmp_path / "second.json").write_bytes(second)
    pkg = {k: v for k, v in json.loads(pkg_xab).items() if k != "signature"}
    forged = d.canonical(dict(pkg, signature=d.b64(d.sign(ROGUE_SEED, d.canonical(pkg)))))
    good_keys = tmp_path / "good-keys.txt"
    good_keys.write_text(f"xab={PUB['xab']}\nxaa={PUB['xaa']}\n")
    wrong_keys = tmp_path / "wrong-keys.txt"
    wrong_keys.write_text(f"xab={PUB['xaa']}\n")

    def steps(run_, root):
        theirs_work = root / "xab-work"
        theirs_work.mkdir()
        (theirs_work / "ours.spiffe-bundle").write_bytes(BUNDLE)
        work = root / "work"
        work.mkdir()
        # The counterparty publishes, hosted, with CA and bundle; we publish and look it up.
        run_("--url", server, "--ours", tmp_path / "pkg-xab.json", "--key", KEY["xab"], "--ca", tmp_path / "xab-ca.pem",
             "--work", theirs_work)
        run_("--url", server, "--ours", tmp_path / "pkg-xaa.json", "--key", KEY["xaa"], "--ca", tmp_path / "xaa-ca.pem",
             "--their-id", "xab", "--theirs", work / "theirs.canonical.json", "--work", work)
        # Again with the keys recorded.
        run_("--url", server, "--ours", tmp_path / "pkg-xaa.json", "--key", KEY["xaa"], "--their-id", "xab",
             "--theirs", work / "theirs.canonical.json", "--keyring", good_keys, "--work", work)
        # A package on disk that the recorded key did not sign, and a CA and bundle beside it.
        (work / "theirs.canonical.json").write_bytes(forged)
        (work / "theirs-ca.pem").write_bytes(b"other CA")
        (work / "theirs.spiffe-bundle").write_bytes(b"other bundle")
        run_("--url", server, "--ours", tmp_path / "pkg-xaa.json", "--their-id", "xab",
             "--theirs", work / "theirs.canonical.json", "--keyring", good_keys, "--work", work)
        # A recorded key that did not sign their package, with nothing on disk.
        w2 = root / "work2"
        w2.mkdir()
        run_("--url", server, "--ours", tmp_path / "pkg-xaa.json", "--their-id", "xab",
             "--theirs", w2 / "theirs.canonical.json", "--keyring", wrong_keys, "--work", w2)
        # Two peerings of one party, hosted: the second replaces the first, as before.
        w3, w4 = root / "ab", root / "ac"
        w3.mkdir()
        w4.mkdir()
        run_("--url", server, "--ours", tmp_path / "first.json", "--key", KEY["xaa"], "--their-id", "xab", "--work", w3)
        run_("--url", server, "--ours", tmp_path / "second.json", "--key", KEY["xaa"], "--their-id", "xac", "--work", w4)
        # A failed package upload after the pointer was accepted.
        Server.fail_puts["xaa/package"] = 1
        run_("--url", server, "--ours", tmp_path / "first.json", "--key", KEY["xaa"], "--their-id", "xab", "--work", w3)
        # Our own host.
        Server.files["/packages/xaa.json"] = first
        run_("--url", server, "--ours", tmp_path / "first.json", "--key", KEY["xaa"], "--package-url",
             f"{server}/packages/xaa.json", "--their-id", "xab", "--work", w3)

    files, state, printed = _same_with_and_without_this_change(server, tmp_path, steps, monkeypatch)
    # The scenarios reached the paths the review named: a package placed under a wrong key,
    # nothing moved aside, no mark file, the second peering replacing the first.
    assert "work2/theirs.canonical.json" in files and not any(".refused" in f for f in files)
    assert not any(f.endswith(d.SINGLE_SLOT_MARK) for f in files)
    assert "WARN    package signature" in printed[4][1]
    assert not any("/packages/" in r for r in state["hosted"])
