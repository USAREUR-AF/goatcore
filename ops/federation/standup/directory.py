#!/usr/bin/env python3
"""The optional directory step of a peering standup, phase 5.

Run only when FED_DIRECTORY_URL is set. The directory holds pointers, not
permissions: an entry says where a party's signed package can be fetched and the
sha256 of that package, until a date, signed by the party itself. This step:

  1. checks that our package is served, byte for byte, at FED_PACKAGE_URL — a
     static file behind TLS on a public host, never on a mesh;
  2. publishes our pointer entry (sovereign_id, package_url, package_sha256,
     expires, signature) signed with the key that signed our package;
  3. looks up the counterparty's pointer, fetches their package from the address
     it gives, checks the digest, checks both signatures against the key bound
     out of band when the keyring has it, and places the package where phase 6
     reads it if nothing is there yet;
  4. prints a compatibility report computed from the two packages — one row per
     fact a partner would otherwise be asked by message, PASS / WARN / FAIL /
     UNKNOWN, each with the fix.

THE INVARIANT. The directory is advisory, never authoritative; being listed grants
nothing (docs/design/peering-directory-service.md §2). It is not an input to the
check: `compose` verifies the counterparty's package against the key bound out of
band, never against anything the directory served. So nothing here can fail a
peering: a directory that is down, refusing or wrong produces a WARN and exit 0.

usage:
  directory.py --url URL --ours FILE [--package-url URL] [--key FILE]
               [--theirs FILE] [--their-id SOVEREIGN_ID] [--keyring FILE] [--work DIR]
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from datetime import date

POINTER_SPEC = "saltlick-pointer/v1"
PACKAGE_SPEC = "federation-package/v1"
OFFER_BLOCKS = ("transport_offer", "data_offer", "cut")
# The fixed transport preference order of federation-package-spec.md §7.
TRANSPORT_PREFERENCE = ("wireguard", "ipsec-fips")
SOVEREIGN_ID_RE = re.compile(r"^[a-z0-9-]{2,16}$")
MAX_PACKAGE_BYTES = 64 << 10
ADVISORY = ("ADVISORY — the directory grants nothing and is not an input to the check. "
            "compose verifies against the key you bound out of band.")


def read_bytes(path: str) -> bytes:
    with open(path, "rb") as f:
        return f.read()



# ---------------------------------------------------------------------------
# Canonical form (RFC 8785 for the value space a package uses: objects, arrays,
# strings, integers, booleans). A float is refused rather than guessed at.
# ---------------------------------------------------------------------------

def _no_floats(v):
    if isinstance(v, float):
        raise ValueError("a package carries no floating-point numbers")
    if isinstance(v, dict):
        for x in v.values():
            _no_floats(x)
    elif isinstance(v, list):
        for x in v:
            _no_floats(x)


def _utf16_key(s: str):
    return s.encode("utf-16-be")


def _canon(v) -> str:
    if isinstance(v, dict):
        items = sorted(v.items(), key=lambda kv: _utf16_key(kv[0]))
        return "{" + ",".join(json.dumps(k, ensure_ascii=False) + ":" + _canon(x) for k, x in items) + "}"
    if isinstance(v, list):
        return "[" + ",".join(_canon(x) for x in v) + "]"
    return json.dumps(v, ensure_ascii=False)


def canonical(v) -> bytes:
    _no_floats(v)
    return _canon(v).encode("utf-8")


# ---------------------------------------------------------------------------
# Ed25519 — the `cryptography` package when present, else the openssl CLI.
# ---------------------------------------------------------------------------

_PKCS8_PREFIX = bytes.fromhex("302e020100300506032b657004220420")


def load_seed(path: str) -> bytes:
    text = read_bytes(path).decode("utf-8").strip()
    if len(text) == 64 and all(c in "0123456789abcdefABCDEF" for c in text):
        return bytes.fromhex(text)
    try:
        b = base64.b64decode(text, validate=True)
    except Exception as e:  # noqa: BLE001 — any decode failure is the same refusal
        raise ValueError(f"{path}: expected an Ed25519 seed as 64 hex characters or 32 bytes base64") from e
    if len(b) != 32:
        raise ValueError(f"{path}: expected an Ed25519 seed as 64 hex characters or 32 bytes base64")
    return b


def _openssl(args, data=None) -> bytes:
    exe = shutil.which("openssl")
    if not exe:
        raise RuntimeError("neither the Python `cryptography` package nor the openssl CLI is available")
    r = subprocess.run([exe, *args], input=data, capture_output=True, check=False)
    if r.returncode != 0:
        raise RuntimeError(f"openssl {' '.join(args[:2])}: {r.stderr.decode(errors='replace').strip()}")
    return r.stdout


def _with_key_file(seed: bytes, fn):
    with tempfile.TemporaryDirectory(prefix="fed-dir-") as d:
        kp = os.path.join(d, "k.der")
        fd = os.open(kp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "wb") as f:
            f.write(_PKCS8_PREFIX + seed)
        return fn(d, kp)


def public_key(seed: bytes) -> bytes:
    try:
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
        from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
        return Ed25519PrivateKey.from_private_bytes(seed).public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    except ImportError:
        der = _with_key_file(seed, lambda d, kp: _openssl(["pkey", "-inform", "DER", "-in", kp, "-pubout", "-outform", "DER"]))
        return der[-32:]


def sign(seed: bytes, msg: bytes) -> bytes:
    try:
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
        return Ed25519PrivateKey.from_private_bytes(seed).sign(msg)
    except ImportError:
        def run(d, kp):
            mp = os.path.join(d, "m")
            with open(mp, "wb") as f:
                f.write(msg)
            return _openssl(["pkeyutl", "-sign", "-keyform", "DER", "-inkey", kp, "-rawin", "-in", mp])
        return _with_key_file(seed, run)


def verify(pub: bytes, msg: bytes, sig: bytes) -> bool:
    try:
        from cryptography.exceptions import InvalidSignature
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
        try:
            Ed25519PublicKey.from_public_bytes(pub).verify(sig, msg)
            return True
        except InvalidSignature:
            return False
    except ImportError:
        spki = bytes.fromhex("302a300506032b6570032100") + pub
        with tempfile.TemporaryDirectory(prefix="fed-dir-") as d:
            paths = {}
            for name, data in (("p", spki), ("m", msg), ("s", sig)):
                paths[name] = os.path.join(d, name)
                with open(paths[name], "wb") as f:
                    f.write(data)
            exe = shutil.which("openssl")
            if not exe:
                raise RuntimeError("neither the Python `cryptography` package nor the openssl CLI is available")
            r = subprocess.run([exe, "pkeyutl", "-verify", "-pubin", "-keyform", "DER", "-inkey", paths["p"],
                                "-rawin", "-in", paths["m"], "-sigfile", paths["s"]], capture_output=True, check=False)
            return r.returncode == 0


def b64(b: bytes) -> str:
    return base64.b64encode(b).decode()


def key_groups(key_b64: str) -> str:
    """The verifying key as it is read aloud: the group count, then groups of four."""
    groups = [key_b64[i:i + 4] for i in range(0, len(key_b64), 4)]
    return f"{len(groups)} groups: " + " ".join(groups)


# ---------------------------------------------------------------------------
# The directory's API (saltlick SPEC.md / README "API").
# ---------------------------------------------------------------------------

def request_statement(method: str, sid: str, body: bytes, key_b64: str | None) -> bytes:
    return ("saltlick-request/v1\n" + method + "\n" + sid + "\n"
            + "sha256:" + hashlib.sha256(body).hexdigest() + "\n"
            + (key_b64 or "-") + "\n").encode()


def _http(method: str, url: str, body: bytes | None, headers: dict, timeout: float):
    req = urllib.request.Request(url, data=body, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:  # noqa: S310 — the URL is the operator's
            return r.status, dict(r.headers), r.read()
    except urllib.error.HTTPError as e:
        return e.code, dict(e.headers or {}), e.read()


def entry_url(base: str, sid: str) -> str:
    return base.rstrip("/") + "/v1/parties/" + urllib.parse.quote(sid, safe="")


# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------

class Report:
    def __init__(self):
        self.rows = []

    def add(self, status, check, detail, fix=""):
        self.rows.append((status, check, detail, fix))

    def print(self, title):
        print(f"\n    {title}")
        for status, check, detail, fix in self.rows:
            print(f"      {status:<7} {check:<18} {detail}")
            if fix:
                print(f"      {'':<7} {'':<18} fix: {fix}")


def _caps(e):
    return (e or {}).get("capabilities") or {}


def compatibility(ours: dict, theirs: dict, their_expired: bool) -> Report:
    """One row per fact a partner would otherwise be asked by message."""
    r = Report()
    a, b = ours.get("sovereign_id", "?"), theirs.get("sovereign_id", "?")
    ca, cb = _caps(ours), _caps(theirs)

    # 0 — the two refusals compose makes on identity.
    if a == b:
        r.add("FAIL", "sovereign_id", f"both parties are `{a}`",
              "a sovereign_id is unique between the two parties; one side corrects its declaration")
    else:
        r.add("PASS", "sovereign_id", f"`{a}` and `{b}`")

    # 1 — versions.
    sa, sb = ours.get("spec"), theirs.get("spec")
    if sa == sb:
        r.add("PASS", "spec", f"both `{sa}`")
    else:
        r.add("FAIL", "spec", f"`{sa}` vs `{sb}`",
              "compose refuses a version mismatch; the side on the older spec upgrades its goat release")
    fa, fb = ca.get("fabric_version"), cb.get("fabric_version")
    if fa == fb:
        r.add("PASS", "fabric_version", f"both `{fa}`")
    else:
        r.add("WARN", "fabric_version", f"`{fa}` vs `{fb}`",
              "the two edge routers peer across versions; agree one fabric version before phase 12")
    ta, tb = ca.get("tool_version"), cb.get("tool_version")
    if ta is None or tb is None:
        r.add("UNKNOWN", "tool_version", "not carried by the package (`tool_version` is reserved: designed, not built)",
              "both sides run `goat --version` and read it out on the call where the digest is compared")
    elif ta == tb:
        r.add("PASS", "tool_version", f"both `{ta}`")
    else:
        r.add("WARN", "tool_version", f"`{ta}` vs `{tb}`",
              "compose with the same release on both sides, or the digests may not match")

    # 2 — transports.
    xa, xb = list(ca.get("transports") or []), list(cb.get("transports") or [])
    common = [t for t in xa if t in xb]
    chosen = next((t for t in TRANSPORT_PREFERENCE if t in common), None)
    if chosen:
        r.add("PASS", "transports", f"{xa} and {xb} share {sorted(common)}; the agreement selects `{chosen}`")
    elif common:
        r.add("FAIL", "transports", f"shared {sorted(common)}, none in the preference order {list(TRANSPORT_PREFERENCE)}",
              "one side adds `wireguard` or `ipsec-fips` to what it can run")
    else:
        r.add("FAIL", "transports", f"{xa} and {xb} share nothing",
              "one side adds a transport the other declares; a party that needs IPsec declares `ipsec-fips` only")

    # 3 — FIPS fabric image.
    pa, pb = ca.get("fabric_fips_image") is True, cb.get("fabric_fips_image") is True
    if pa and pb:
        r.add("PASS", "fabric_fips_image", "both true")
    elif pa or pb:
        only = a if pa else b
        r.add("WARN", "fabric_fips_image", f"only `{only}` runs a FIPS fabric image",
              "the FIPS claim is one-sided: the link is not FIPS on both ends; record that in the agreement review")
    else:
        r.add("WARN", "fabric_fips_image", "neither side runs a FIPS fabric image",
              "acceptable only if neither party's policy requires FIPS")

    # 4 — post-quantum.
    qa, qb = ca.get("pqc_capable") is True, cb.get("pqc_capable") is True
    if qa and qb:
        r.add("PASS", "pqc_capable", "both true")
    else:
        r.add("WARN", "pqc_capable", f"`{a}` {qa}, `{b}` {qb}",
              "post-quantum protection applies only when both sides are capable")

    # 5 — trust domains, the SPIRE fact.
    da = (ours.get("identity") or {}).get("trust_domain")
    db = (theirs.get("identity") or {}).get("trust_domain")
    if not da or not db:
        r.add("FAIL", "trust_domain", f"`{da}` / `{db}`", "both packages carry identity.trust_domain")
    elif da == db:
        r.add("FAIL", "trust_domain", f"both `{da}`",
              "compose refuses the same trust domain; two sovereigns have two SPIFFE trust domains")
    else:
        r.add("PASS", "trust_domain", f"`{da}` and `{db}` — phase 13 federates them")

    # 6 — reachability (designed `inbound` capability, ADR 1218).
    ia, ib = ca.get("inbound"), cb.get("inbound")
    if ia is None or ib is None:
        r.add("UNKNOWN", "inbound", "not carried (`inbound` is designed, not built)",
              "settle on the call which edge accepts inbound UDP; if neither can, one side runs a "
              "peering forwarder before issuing its package (ADR 1218)")
    elif ia == "none" and ib == "none":
        r.add("FAIL", "inbound", "both `none`",
              "one side must run a peering forwarder before issuing its package (ADR 1218)")
    else:
        r.add("PASS", "inbound", f"`{a}` {ia}, `{b}` {ib}")

    # expiry, reported never enforced (the directory serves expired entries, flagged).
    exp = theirs.get("expires", "")
    if their_expired or (exp and exp < date.today().isoformat()):
        r.add("WARN", "expires", f"`{b}`'s entry expired {exp}", f"ask `{b}` to republish; compose refuses an expired package")
    return r


def read_keyring(path: str | None) -> dict:
    keys = {}
    if path and os.path.isfile(path):
        for line in read_bytes(path).decode("utf-8").splitlines():
            line = "".join(line.split())
            if not line or line.startswith("#") or "=" not in line:
                continue
            sid, key = line.split("=", 1)
            keys[sid] = key
    return keys




def sha256_of(b: bytes) -> str:
    return "sha256:" + hashlib.sha256(b).hexdigest()


def pointer(sid: str, package_url: str, package_bytes: bytes, expires: str) -> dict:
    """The unsigned pointer entry for a package served at package_url."""
    return {"spec": POINTER_SPEC, "sovereign_id": sid, "package_url": package_url,
            "package_sha256": sha256_of(package_bytes), "expires": expires}


def _get(url: str, timeout: float, accept="application/json"):
    req = urllib.request.Request(url, method="GET", headers={"Accept": accept})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:  # noqa: S310 — operator- or pointer-supplied https URL
            return r.status, {k.lower(): v for k, v in r.headers.items()}, r.read(MAX_PACKAGE_BYTES + 1)
    except urllib.error.HTTPError as e:
        return e.code, {k.lower(): v for k, v in (e.headers or {}).items()}, e.read(MAX_PACKAGE_BYTES + 1)


def _https_or_local(url: str) -> bool:
    p = urllib.parse.urlparse(url)
    return p.scheme == "https" or (p.scheme == "http" and p.hostname in ("localhost", "127.0.0.1", "::1"))


def publish(url, ours_raw: bytes, package_url: str | None, key_path: str | None, work: str | None,
            timeout: float) -> list:
    """Publish our pointer. Returns warnings; an empty list means it was published."""
    try:
        pkg = json.loads(ours_raw)
    except ValueError as e:
        return [f"our package is not JSON ({e}); nothing published"]
    sid = pkg.get("sovereign_id", "")
    if not package_url:
        return ["FED_PACKAGE_URL is not set, so there is no address to point at; nothing published. "
                f"Serve {sid}'s package as a static file behind TLS on a public host and set it"]
    if not _https_or_local(package_url):
        return [f"FED_PACKAGE_URL {package_url} is not https; nothing published"]
    # The pointer must point at what we would send. Fetch it back and compare.
    try:
        status, _, served = _get(package_url, timeout)
    except (OSError, urllib.error.URLError) as e:
        return [f"our package is not reachable at {package_url} ({e}); place {sid}'s package there; nothing published"]
    if status != 200 or served != ours_raw:
        return [f"{package_url} does not serve our package byte for byte (HTTP {status}); "
                "copy the file phase 3 wrote there unchanged; nothing published"]
    if not key_path:
        return ["FED_SIGNING_KEY is not set, so the pointer cannot be signed; nothing published"]
    try:
        seed = load_seed(key_path)
        pub = public_key(seed)
        if canonical(pkg) != ours_raw:
            return ["our package is not in canonical form, so its signature cannot be checked here; nothing published"]
        sig = base64.b64decode(pkg.get("signature", ""))
        if not verify(pub, canonical({k: v for k, v in pkg.items() if k != "signature"}), sig):
            return ["FED_SIGNING_KEY did not sign our package; nothing published"]
        entry = pointer(sid, package_url, ours_raw, pkg.get("expires", ""))
        body = canonical(dict(entry, signature=b64(sign(seed, canonical(entry)))))
    except (OSError, ValueError, RuntimeError) as e:
        return [f"could not sign the pointer: {e}; nothing published"]
    if work:
        try:
            with open(os.path.join(work, "ours.pointer.json"), "wb") as f:
                f.write(body)
        except OSError:
            pass  # a convenience copy for the operator; the pointer is already built
    key_b64 = b64(pub)
    headers = {"Content-Type": "application/json", "X-Saltlick-Key": key_b64,
               "X-Saltlick-Signature": b64(sign(seed, request_statement("PUT", sid, body, key_b64)))}
    try:
        status, _, resp = _http("PUT", entry_url(url, sid), body, headers, timeout)
    except (OSError, urllib.error.URLError) as e:
        return [f"directory unreachable at {url}: {e}"]
    text = resp.decode(errors="replace").strip()
    if status not in (200, 201):
        return [f"publish refused, HTTP {status}: {text[:300]}"]
    try:
        info = json.loads(text)
        how = "claimed on first use" if info.get("first_use") else ("rotated" if info.get("rotated") else "replaced")
    except ValueError:
        how = "accepted"
    print(f"    published `{sid}` → {package_url} ({how})")
    print(f"    our verifying key, to read aloud: {key_groups(key_b64)}")
    return []


def lookup(url, sid: str, bound_key: str | None, timeout: float, report: "Report"):
    """Our counterparty's package, through their pointer. Returns (bytes, dict, fetched_through_a_pointer)."""
    try:
        status, headers, body = _get(entry_url(url, sid), timeout)
    except (OSError, urllib.error.URLError) as e:
        report.add("WARN", "lookup", f"directory unreachable at {url}: {e}")
        return None, None, False
    if status != 200:
        report.add("WARN", "lookup", f"no entry for `{sid}` (HTTP {status})",
                   "until they publish one, the package moves out of band")
        return None, None, False
    try:
        entry = json.loads(body)
    except ValueError:
        report.add("WARN", "lookup", f"the entry for `{sid}` is not JSON")
        return None, None, False
    pub = None
    if bound_key:
        try:
            pub = base64.b64decode(bound_key, validate=True)
        except ValueError:
            pub = None
    if headers.get("x-saltlick-expired", "").lower() == "true" or entry.get("expires", "9999") < date.today().isoformat():
        report.add("WARN", "expires", f"`{sid}`'s entry expired {entry.get('expires')}", f"ask `{sid}` to republish")

    def signed_by_bound_key(doc: dict, what: str):
        if pub is None:
            report.add("UNKNOWN", what, f"no key for `{sid}` in the keyring yet",
                       "read their key back by voice, group count first, and record it in the keyring")
            return
        try:
            ok = len(pub) == 32 and verify(pub, canonical({k: v for k, v in doc.items() if k != "signature"}),
                                           base64.b64decode(doc.get("signature", "")))
        except (ValueError, RuntimeError):
            ok = False
        if ok:
            report.add("PASS", what, "verifies under the key you bound out of band")
        else:
            report.add("WARN", what, "does NOT verify under the key you bound out of band",
                       "trust the key you read back, not the directory; tell the counterparty")

    if entry.get("spec") == PACKAGE_SPEC:
        # An older entry form: the recognition-only package itself.
        report.add("PASS", "lookup", f"`{sid}` lists a recognition-only package (no pointer)")
        signed_by_bound_key(entry, "entry signature")
        return body, entry, False
    if entry.get("spec") != POINTER_SPEC:
        report.add("WARN", "lookup", f"`{sid}`'s entry has spec {entry.get('spec')!r}, which this step does not read")
        return None, None, False
    if entry.get("sovereign_id") != sid:
        report.add("WARN", "lookup", f"the entry for `{sid}` names `{entry.get('sovereign_id')}`")
        return None, None, False
    signed_by_bound_key(entry, "pointer signature")
    purl = entry.get("package_url", "")
    if not _https_or_local(purl):
        report.add("WARN", "package_url", f"{purl!r} is not https; not fetched")
        return None, None, False
    try:
        status, _, pbody = _get(purl, timeout)
    except (OSError, urllib.error.URLError) as e:
        report.add("WARN", "package fetch", f"{purl}: {e}", "ask them to serve the package there, or send it out of band")
        return None, None, False
    if status != 200 or len(pbody) > MAX_PACKAGE_BYTES:
        report.add("WARN", "package fetch", f"{purl}: HTTP {status}, {len(pbody)} bytes")
        return None, None, False
    if sha256_of(pbody) != entry.get("package_sha256"):
        report.add("FAIL", "package digest", f"{purl} serves {sha256_of(pbody)}, the pointer says {entry.get('package_sha256')}",
                   "do not use it; ask them which is current")
        return None, None, False
    report.add("PASS", "package digest", f"{purl} serves the package the pointer names ({entry['package_sha256'][:23]}…)")
    try:
        pkg = json.loads(pbody)
    except ValueError:
        report.add("FAIL", "package", "the fetched package is not JSON")
        return None, None, False
    if pkg.get("sovereign_id") != sid:
        report.add("FAIL", "package", f"the fetched package is `{pkg.get('sovereign_id')}`, not `{sid}`")
        return None, None, False
    signed_by_bound_key(pkg, "package signature")
    return pbody, pkg, True


def transport_rows(ours: dict, theirs: dict, r: "Report"):
    """When both offer a link, the listener rule compose applies (§6 step 4b)."""
    ta, tb = ours.get("transport_offer"), theirs.get("transport_offer")
    if not (ta and tb):
        return
    if not (ta.get("listener") is True or tb.get("listener") is True):
        r.add("FAIL", "listener", "neither package declares listener: true",
              "one side accepts inbound — directly or through a peering forwarder (ADR 1218) — and reissues its package")
    else:
        who = ours.get("sovereign_id") if ta.get("listener") is True else theirs.get("sovereign_id")
        r.add("PASS", "listener", f"`{who}` listens")


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--url", required=True)
    ap.add_argument("--ours", required=True)
    ap.add_argument("--package-url")
    ap.add_argument("--key")
    ap.add_argument("--theirs")
    ap.add_argument("--their-id")
    ap.add_argument("--keyring")
    ap.add_argument("--work")
    ap.add_argument("--timeout", type=float, default=10.0)
    a = ap.parse_args(argv)

    print(f"\n    Directory step (optional): {a.url}")
    print(f"    {ADVISORY}")
    warnings = []
    if not _https_or_local(a.url):
        warnings.append("the directory URL is not https; a reader on the path could alter what you fetch")

    try:
        ours_raw = read_bytes(a.ours)
        ours = json.loads(ours_raw)
    except (OSError, ValueError) as e:
        print(f"    WARN    our package is not readable ({e}); the directory step is skipped")
        return 0
    warnings += publish(a.url, ours_raw, a.package_url, a.key, a.work, a.timeout)

    theirs_on_disk = None
    if a.theirs and os.path.isfile(a.theirs):
        try:
            theirs_on_disk = json.loads(read_bytes(a.theirs))
        except ValueError:
            warnings.append(f"{a.theirs} is not JSON")
    their_id = a.their_id or (theirs_on_disk or {}).get("sovereign_id")
    if their_id and not SOVEREIGN_ID_RE.match(their_id):
        warnings.append(f"`{their_id}` is not a sovereign_id; nothing looked up")
        their_id = None

    if not their_id:
        warnings.append("the counterparty's sovereign_id is not known: set FED_COUNTERPARTY_ID")
    else:
        checks = Report()
        keyring = read_keyring(a.keyring)
        body, theirs, via_pointer = lookup(a.url, their_id, keyring.get(their_id), a.timeout, checks)
        checks.print(f"Lookup of `{their_id}`")
        if theirs is not None:
            if a.work:
                try:
                    with open(os.path.join(a.work, "theirs.fetched.json"), "wb") as f:
                        f.write(body)
                except OSError:
                    pass  # a convenience copy for the operator; nothing reads it
            if via_pointer and theirs_on_disk is None and a.theirs:
                # A full package fetched through their pointer: place it where phase 6 reads
                # it. compose still verifies it against the key bound out of band.
                try:
                    fd = os.open(a.theirs, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                    with os.fdopen(fd, "wb") as f:
                        f.write(body)
                    print(f"\n    placed `{their_id}`'s package at {a.theirs} — phase 6 composes it against the key you bound")
                except OSError as e:
                    warnings.append(f"could not place their package at {a.theirs}: {e}")
            elif via_pointer and theirs_on_disk is not None and read_bytes(a.theirs) != body:
                warnings.append(f"{a.theirs} differs from the package `{their_id}` serves; the file on disk is what composes")
            report = compatibility(ours, theirs, False)
            transport_rows(ours, theirs, report)
            report.print("COMPATIBILITY REPORT — computed from the two packages (advisory)")

    for w in warnings:
        print(f"    WARN    {w}")
    print("\n    Still moved directly between the two parties: the CA certificate and the SPIFFE bundle.\n"
          "    The out-of-band act that remains: read the verifying-key fingerprint, and after\n"
          "    phase 6 the agreement digest, aloud to the counterparty. The directory replaces neither.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except SystemExit:
        raise
    except Exception as e:  # noqa: BLE001 — the directory step never fails a peering
        print(f"    WARN    directory step error: {e}; carrying on without it")
        sys.exit(0)
