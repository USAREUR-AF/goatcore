#!/usr/bin/env python3
"""The directory step of a peering standup, phase 5.

Run only when FED_DIRECTORY_URL is set. The directory holds pointers, not
permissions: an entry says where a party's signed package can be fetched and the
sha256 of that package, until a date, signed by the party itself. This step:

  1. publishes our pointer entry (sovereign_id, package_url, package_sha256,
     expires, signature) signed with the key that signed our package. With
     FED_PACKAGE_URL set, the pointer names our own public host, which must
     serve our package byte for byte. Unset, the directory hosts it: we upload
     our package to <directory>/v1/parties/<us>/package and the pointer names
     that, so a party behind NAT needs no public host;
  2. uploads our CA certificate to <directory>/v1/parties/<us>/ca: the
     certificate in FED_CA_CERT, else in the profile's ca-roots.pem, whose
     SHA-256 is our package's identity.ca_fingerprint;
  3. looks up the counterparty's pointer, fetches their package from the address
     it gives, checks the digest, checks both signatures against the key bound
     on the call when the keyring has it, and places the package where phase 6
     reads it if nothing is there yet; fetches their CA certificate, checks it
     against their package's identity.ca_fingerprint, and places it at
     $FED_WORK/theirs-ca.pem if nothing is there yet;
  4. prints their verifying key and CA fingerprint in groups of four for the
     voice call, and a compatibility report computed from the two packages —
     one row per fact a partner would otherwise be asked by message,
     PASS / WARN / FAIL / UNKNOWN, each with the fix.

SEVERAL PEERINGS (--per-counterparty, or FED_DIRECTORY_PER_COUNTERPARTY=1; off by
default). A package carries one transport offer, so a party with two peerings signs
two packages, and the single slot <directory>/v1/parties/<us>/package lets the second
replace the first. With the flag on, this step also uploads our package to
<directory>/v1/parties/<us>/packages/<them>, one slot per counterparty, and looks
the counterparty's package up at <directory>/v1/parties/<them>/packages/<us> first,
checking the upload signature, whose resource line names us, under their key; it
falls back to their pointer when they host no package for us. The single slot and
the pointer are written only when they are empty or hold this same peering's
package (the same transport_offer), so they keep serving the first peering to a
counterparty on an older release. The package format is unchanged
(docs/design/federation-package-spec.md §11.1).

THE INVARIANT. The directory is advisory, never authoritative; being listed grants
nothing (docs/design/peering-directory-service.md §2). It is not an input to the
check: `compose` verifies the counterparty's package against the key read back on the
call, never against anything the directory served. So nothing here can fail a
peering: a directory that is down, refusing or wrong produces a WARN and exit 0.

usage:
  directory.py --url URL --ours FILE [--package-url URL] [--key FILE] [--ca FILE]
               [--theirs FILE] [--their-id SOVEREIGN_ID] [--keyring FILE] [--work DIR]
               [--per-counterparty]
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import http.client
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
            "compose verifies against the key you read back on the call.")


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


def fingerprint_groups(fp: str) -> str:
    """A `sha256:<hex>` CA fingerprint as it is read aloud: the hex in groups of four."""
    return key_groups(fp.removeprefix("sha256:"))


# ---------------------------------------------------------------------------
# CA certificates. identity.ca_fingerprint is `sha256:` over the DER of the
# trust anchor (federation-package-spec.md §5.1), so a certificate is matched to
# a package by that digest, never by position in a bundle.
# ---------------------------------------------------------------------------

_PEM_CERT_RE = re.compile(rb"-----BEGIN CERTIFICATE-----\s*(.*?)\s*-----END CERTIFICATE-----", re.S)


def pem_certificates(pem: bytes) -> list:
    """The DER of each CERTIFICATE block in pem; a block that is not base64 is skipped."""
    out = []
    for m in _PEM_CERT_RE.finditer(pem):
        try:
            out.append(base64.b64decode(b"".join(m.group(1).split()), validate=True))
        except ValueError:
            continue
    return out


def der_to_pem(der: bytes) -> bytes:
    b = base64.b64encode(der).decode()
    return ("-----BEGIN CERTIFICATE-----\n" + "\n".join(b[i:i + 64] for i in range(0, len(b), 64))
            + "\n-----END CERTIFICATE-----\n").encode()


def anchor_for(pem: bytes, fingerprint: str):
    """The certificate in pem whose SHA-256 is fingerprint, as PEM; None if there is none."""
    for der in pem_certificates(pem):
        if sha256_of(der) == fingerprint:
            return der_to_pem(der)
    return None


def profile_ca_roots() -> str:
    """The active goat profile's ca-roots.pem, resolved as `goat` resolves it."""
    pdir = os.environ.get("GOAT_PROFILE_DIR") or os.path.join(os.path.expanduser("~"), ".goat")
    ctx = os.environ.get("GOAT_CONTEXT", "")
    if not ctx:
        try:
            for line in read_bytes(os.path.join(pdir, "config.toml")).decode("utf-8").splitlines():
                k, _, v = line.partition("=")
                if k.strip() == "current_context":
                    ctx = v.strip().strip('"').strip("'")
        except OSError:
            pass  # no config.toml: `goat` falls back to the default context too
    return os.path.join(pdir, "contexts", ctx or "default", "ca-roots.pem")


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


def _https(url: str) -> bool:
    # saltlick refuses a pointer whose package_url is not https, and the
    # pointer's package_url is built from the directory URL, so nothing here
    # accepts plain http. A private CA is trusted through SSL_CERT_FILE.
    return urllib.parse.urlparse(url).scheme == "https"


def signed_put(url: str, sid: str, resource: str, body: bytes, seed: bytes, content_type: str, timeout: float):
    """PUT body to <url>/v1/parties/<resource>, signed as the saltlick write rule requires."""
    key_b64 = b64(public_key(seed))
    path = resource if resource == sid else sid + "/" + resource
    headers = {"Content-Type": content_type, "X-Saltlick-Key": key_b64,
               "X-Saltlick-Signature": b64(sign(seed, request_statement("PUT", path, body, key_b64)))}
    target = entry_url(url, sid) + ("" if resource == sid else "/" + resource)
    return _http("PUT", target, body, headers, timeout)


def counterparty_slot(sid: str, their_id: str) -> str:
    """The resource of the package `sid` signed for `their_id`: <sid>/packages/<their_id>."""
    return f"{sid}/packages/{their_id}"


SINGLE_SLOT_MARK = "directory-single-slot.sha256"


def single_slot_is_ours(url, sid: str, pkg: dict, package_url: str, hosted: bool, work: str | None,
                        timeout: float):
    """Whether this peering may write the pointer (and, hosted, the single slot).

    Returns (verdict, reason). verdict is True when the slot is free or already this
    peering's, False when another peering's package holds it, None when the directory
    could not tell us. Already this peering's means: the bytes it holds are the bytes
    this peering last wrote (recorded in <work>/directory-single-slot.sha256, the work
    directory being per peering), or they carry our transport_offer. With our own host
    (not hosted), the pointer is ours when it is absent or names our package_url."""
    if not hosted:
        try:
            status, _, body = _get(entry_url(url, sid), timeout)
        except (OSError, urllib.error.URLError, http.client.HTTPException) as e:
            return None, f"could not read our entry: {e}"
        if status == 404:
            return True, ""
        if status != 200:
            return None, f"reading our entry answered HTTP {status}"
        try:
            entry = json.loads(body)
        except ValueError:
            return True, ""
        if not isinstance(entry, dict) or entry.get("spec") != POINTER_SPEC:
            return True, ""
        return entry.get("package_url") == package_url, "our pointer names another peering's package"
    try:
        status, _, served = _get(entry_url(url, sid) + "/package", timeout)
    except (OSError, urllib.error.URLError, http.client.HTTPException) as e:
        return None, f"could not read {entry_url(url, sid)}/package: {e}"
    if status == 404:
        return True, ""
    if status != 200:
        return None, f"reading {entry_url(url, sid)}/package answered HTTP {status}"
    if work:
        try:
            if read_bytes(os.path.join(work, SINGLE_SLOT_MARK)).decode().strip() == sha256_of(served):
                return True, ""
        except (OSError, UnicodeDecodeError):
            pass  # no mark: this peering has not written the single slot from this work directory
    try:
        held = json.loads(served)
    except ValueError:
        return True, ""  # not a package at all: nothing of another peering's to keep
    if not isinstance(held, dict):
        return True, ""
    return held.get("transport_offer") == pkg.get("transport_offer"), "the single package slot holds another peering's package"


def mark_single_slot(work: str | None, raw: bytes):
    """Record that this peering wrote the single slot, so a reissue with a changed offer
    still counts as this peering's."""
    if not work:
        return
    try:
        with open(os.path.join(work, SINGLE_SLOT_MARK), "w", encoding="utf-8") as f:
            f.write(sha256_of(raw) + "\n")
    except OSError:
        pass  # a convenience for the next run; the transport_offer comparison still applies


def publish(url, ours_raw: bytes, package_url: str | None, key_path: str | None, work: str | None,
            timeout: float, ca_path: str | None = None, their_id: str | None = None,
            per_counterparty: bool = False) -> list:
    """Publish our pointer, and with no package_url our package; then our CA.
    With per_counterparty and their_id, also our package for that counterparty, and the
    pointer and single package only when they are free or already this peering's.
    Returns warnings; an empty list means everything was published."""
    try:
        pkg = json.loads(ours_raw)
    except ValueError as e:
        return [f"our package is not JSON ({e}); nothing published"]
    sid = pkg.get("sovereign_id", "")
    if not SOVEREIGN_ID_RE.match(sid):
        return [f"our package's sovereign_id {sid!r} is not a sovereign_id; nothing published"]
    hosted = not package_url
    if hosted:
        package_url = entry_url(url, sid) + "/package"
    elif not _https(package_url):
        return [f"FED_PACKAGE_URL {package_url} is not https; nothing published"]
    else:
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
    slot = per_counterparty and bool(their_id)
    write_single = True
    if slot:
        mine, why = single_slot_is_ours(url, sid, pkg, package_url, hosted, work, timeout)
        if mine is None:
            return [f"the directory at {url} did not say whose package the single slot holds ({why}); nothing published"]
        write_single = mine
    if not write_single:
        # Another peering's package holds the single slot, or the pointer names it.
        # Leave both: a counterparty on an older release reads them for that peering.
        print(f"    {why}; the pointer and single slot are left as they are, "
              f"and `{their_id}` reads ours from {entry_url(url, sid)}/packages/{their_id}")
        how = None
    else:
        # The entry first: its first publish binds the key, and the directory refuses
        # a hosted file for a name no entry has claimed.
        try:
            status, _, resp = signed_put(url, sid, sid, body, seed, "application/json", timeout)
        except (OSError, urllib.error.URLError, RuntimeError) as e:
            return [f"directory unreachable at {url}: {e}"]
        text = resp.decode(errors="replace").strip()
        if status not in (200, 201):
            return [f"publish refused, HTTP {status}: {text[:300]}"]
        try:
            info = json.loads(text)
            how = "claimed on first use" if info.get("first_use") else ("rotated" if info.get("rotated") else "replaced")
        except ValueError:
            how = "accepted"
    warnings = []
    if slot:
        warnings += publish_slot(url, sid, their_id, ours_raw, seed, timeout)
    if hosted and write_single:
        try:
            status, _, resp = signed_put(url, sid, "package", ours_raw, seed, "application/json", timeout)
        except (OSError, urllib.error.URLError, RuntimeError) as e:
            status, resp = 0, str(e).encode()
        if status not in (200, 201):
            warnings.append(f"package upload refused, HTTP {status}: {resp.decode(errors='replace').strip()[:300]}; "
                            "the pointer names a package the directory does not hold")
        else:
            try:
                status, _, served = _get(package_url, timeout)
            except (OSError, urllib.error.URLError) as e:
                status, served = 0, str(e).encode()
            if status != 200 or served != ours_raw:
                warnings.append(f"{package_url} does not serve our package byte for byte after upload (HTTP {status})")
            else:
                print(f"    uploaded `{sid}`'s package to the directory: {package_url}")
                if slot:
                    mark_single_slot(work, ours_raw)
    if how:
        print(f"    published `{sid}` → {package_url} ({how})")
    print(f"    our verifying key, to read aloud: {key_groups(key_b64)}")
    warnings += publish_ca(url, sid, pkg, seed, ca_path, timeout)
    warnings += publish_bundle(url, sid, seed, work, timeout)
    return warnings


def publish_slot(url, sid: str, their_id: str, ours_raw: bytes, seed: bytes, timeout: float) -> list:
    """Upload our package for one counterparty to <directory>/v1/parties/<sid>/packages/<their_id>
    and read it back byte for byte. Another counterparty's slot is never touched."""
    resource = counterparty_slot(sid, their_id)
    target = entry_url(url, sid) + "/packages/" + urllib.parse.quote(their_id, safe="")
    try:
        status, _, resp = signed_put(url, sid, f"packages/{their_id}", ours_raw, seed, "application/json", timeout)
    except (OSError, urllib.error.URLError, RuntimeError, http.client.HTTPException) as e:
        return [f"package upload for `{their_id}` failed: {e}"]
    if status in (404, 405):
        return [f"the directory does not host a package per counterparty (HTTP {status} for {resource}); "
                f"it runs an older saltlick release, so `{their_id}` can read only the single package slot"]
    if status not in (200, 201):
        return [f"package upload for `{their_id}` refused, HTTP {status}: {resp.decode(errors='replace').strip()[:300]}"]
    try:
        status, _, served = _get(target, timeout)
    except (OSError, urllib.error.URLError, http.client.HTTPException) as e:
        status, served = 0, str(e).encode()
    if status != 200 or served != ours_raw:
        return [f"{target} does not serve our package byte for byte after upload (HTTP {status})"]
    print(f"    uploaded `{sid}`'s package for `{their_id}` to the directory: {target}")
    return []


def publish_bundle(url, sid: str, seed: bytes, work: str | None, timeout: float) -> list:
    """Upload our SPIFFE trust bundle, when phase 2 exported one (SPIRE federation on)."""
    path = os.path.join(work, "ours.spiffe-bundle") if work else ""
    if not path or not os.path.isfile(path):
        return []
    try:
        body = read_bytes(path)
        status, _, resp = signed_put(url, sid, "spiffe-bundle", body, seed, "application/json", timeout)
    except (OSError, urllib.error.URLError, RuntimeError) as e:
        return [f"SPIFFE bundle upload failed: {e}"]
    if status not in (200, 201):
        return [f"SPIFFE bundle upload refused, HTTP {status}: {resp.decode(errors='replace').strip()[:300]}"]
    print(f"    uploaded our SPIFFE trust bundle to the directory: {entry_url(url, sid)}/spiffe-bundle")
    return []


def publish_ca(url, sid: str, pkg: dict, seed: bytes, ca_path: str | None, timeout: float) -> list:
    """Upload the CA certificate whose SHA-256 is our identity.ca_fingerprint."""
    fp = (pkg.get("identity") or {}).get("ca_fingerprint", "")
    source = ca_path or profile_ca_roots()
    try:
        pem = read_bytes(source)
    except OSError:
        how = "FED_CA_CERT" if ca_path else "FED_CA_CERT is not set and the profile's ca-roots.pem"
        return [f"{how} ({source}) is not readable; our CA certificate was not uploaded. "
                "Set FED_CA_CERT to our CA certificate (PEM)"]
    anchor = anchor_for(pem, fp)
    if anchor is None:
        return [f"no certificate in {source} has our package's identity.ca_fingerprint {fp}; "
                "our CA certificate was not uploaded. Set FED_CA_CERT to the CA the package names"]
    try:
        status, _, resp = signed_put(url, sid, "ca", anchor, seed, "application/pem-certificate-chain", timeout)
    except (OSError, urllib.error.URLError, RuntimeError) as e:
        return [f"CA upload failed: {e}"]
    if status not in (200, 201):
        return [f"CA upload refused, HTTP {status}: {resp.decode(errors='replace').strip()[:300]}"]
    print(f"    uploaded our CA certificate to the directory: {entry_url(url, sid)}/ca")
    print(f"    our CA fingerprint, to read aloud: {fingerprint_groups(fp)}")
    return []


def lookup(url, sid: str, bound_key: str | None, timeout: float, report: "Report"):
    """Our counterparty's package, through their pointer.
    Returns (bytes, dict, fetched_through_a_pointer, the key the directory has bound for them)."""
    try:
        status, headers, body = _get(entry_url(url, sid), timeout)
    except (OSError, urllib.error.URLError) as e:
        report.add("WARN", "lookup", f"directory unreachable at {url}: {e}")
        return None, None, False, None
    if status != 200:
        report.add("WARN", "lookup", f"no entry for `{sid}` (HTTP {status})",
                   "run phase 5 again after they run theirs")
        return None, None, False, None
    dir_key = headers.get("x-saltlick-bound-key")
    try:
        entry = json.loads(body)
    except ValueError:
        report.add("WARN", "lookup", f"the entry for `{sid}` is not JSON")
        return None, None, False, None
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
            report.add("PASS", what, "verifies under the key you read back on the call")
        else:
            report.add("WARN", what, "does NOT verify under the key you read back on the call",
                       "trust the key you read back, not the directory; tell the counterparty")

    if entry.get("spec") == PACKAGE_SPEC:
        # An older entry form: the recognition-only package itself.
        report.add("PASS", "lookup", f"`{sid}` lists a recognition-only package (no pointer)")
        signed_by_bound_key(entry, "entry signature")
        return body, entry, False, dir_key
    if entry.get("spec") != POINTER_SPEC:
        report.add("WARN", "lookup", f"`{sid}`'s entry has spec {entry.get('spec')!r}, which this step does not read")
        return None, None, False, None
    if entry.get("sovereign_id") != sid:
        report.add("WARN", "lookup", f"the entry for `{sid}` names `{entry.get('sovereign_id')}`")
        return None, None, False, None
    signed_by_bound_key(entry, "pointer signature")
    purl = entry.get("package_url", "")
    if not _https(purl):
        report.add("WARN", "package_url", f"{purl!r} is not https; not fetched")
        return None, None, False, None
    try:
        status, _, pbody = _get(purl, timeout)
    except (OSError, urllib.error.URLError) as e:
        report.add("WARN", "package fetch", f"{purl}: {e}", "ask them to run phase 5, which uploads it")
        return None, None, False, None
    if status != 200 or len(pbody) > MAX_PACKAGE_BYTES:
        report.add("WARN", "package fetch", f"{purl}: HTTP {status}, {len(pbody)} bytes")
        return None, None, False, None
    if sha256_of(pbody) != entry.get("package_sha256"):
        report.add("FAIL", "package digest", f"{purl} serves {sha256_of(pbody)}, the pointer says {entry.get('package_sha256')}",
                   "do not use it; ask them which is current")
        return None, None, False, None
    report.add("PASS", "package digest", f"{purl} serves the package the pointer names ({entry['package_sha256'][:23]}…)")
    try:
        pkg = json.loads(pbody)
    except ValueError:
        report.add("FAIL", "package", "the fetched package is not JSON")
        return None, None, False, None
    if pkg.get("sovereign_id") != sid:
        report.add("FAIL", "package", f"the fetched package is `{pkg.get('sovereign_id')}`, not `{sid}`")
        return None, None, False, None
    signed_by_bound_key(pkg, "package signature")
    return pbody, pkg, True, dir_key


def lookup_slot(url, sid: str, us: str, bound_key: str | None, timeout: float, report: "Report"):
    """The package `sid` signed for us, from <directory>/v1/parties/<sid>/packages/<us>.
    Its upload signature names the slot, so under their key it shows they signed this
    package for us and not for another counterparty. Returns lookup()'s tuple, or None
    when they host no package for us (the caller falls back to their pointer)."""
    resource = counterparty_slot(sid, us)
    target = entry_url(url, sid) + "/packages/" + urllib.parse.quote(us, safe="")
    stop = (None, None, False, None)
    try:
        status, headers, body = _get(target, timeout)
    except (OSError, urllib.error.URLError, http.client.HTTPException) as e:
        # Not a fall-back case: their pointer may name the package of another of their peerings.
        report.add("WARN", "package for us", f"directory unreachable at {url}: {e}", "run phase 5 again once the directory answers")
        return stop
    if status == 404:
        report.add("UNKNOWN", "package for us", f"`{sid}` hosts no package for `{us}` (HTTP 404); reading their pointer",
                   "a counterparty with one peering, or on an older release, publishes only the single package")
        return None
    if status != 200:
        report.add("WARN", "package for us", f"{target}: HTTP {status}; their pointer is not read in its place",
                   "run phase 5 again; their pointer may name the package of another of their peerings")
        return stop
    dir_key = headers.get("x-saltlick-bound-key")
    if len(body) > MAX_PACKAGE_BYTES:
        report.add("FAIL", "package for us", f"{target}: {len(body)} bytes")
        return None, None, False, dir_key
    if headers.get("x-saltlick-resource", resource) != resource:
        report.add("FAIL", "package for us", f"{target} is served as `{headers.get('x-saltlick-resource')}`, not `{resource}`",
                   "do not use it; ask them to run phase 5 again")
        return None, None, False, dir_key
    pub = None
    if bound_key:
        try:
            pub = base64.b64decode(bound_key, validate=True)
        except ValueError:
            pub = b""
        if len(pub) != 32:
            report.add("FAIL", "slot signature", f"the keyring's key for `{sid}` is not 32 bytes of base64",
                       "correct the keyring line from the key read back on the call")
            return None, None, False, dir_key
    if pub is None:
        report.add("UNKNOWN", "slot signature", f"no key for `{sid}` in the keyring yet",
                   "read their key back by voice, group count first, and record it in the keyring")
    else:
        try:
            ok = (headers.get("x-saltlick-key", "") == bound_key
                  and verify(pub, request_statement("PUT", resource, body, bound_key),
                             base64.b64decode(headers.get("x-saltlick-signature", ""))))
        except (ValueError, RuntimeError, OSError):
            ok = False
        if not ok:
            report.add("FAIL", "slot signature", f"the package at {resource} was not uploaded under the key you read back",
                       "do not use it; trust the key you read back, not the directory; tell the counterparty")
            return None, None, False, dir_key
        report.add("PASS", "slot signature", f"`{sid}` uploaded this package for `{us}` under the key you read back")
    try:
        pkg = json.loads(body)
    except ValueError:
        pkg = None
    if not isinstance(pkg, dict):
        report.add("FAIL", "package for us", "the fetched package is not a JSON object")
        return None, None, False, dir_key
    if pkg.get("sovereign_id") != sid:
        report.add("FAIL", "package for us", f"the fetched package is `{pkg.get('sovereign_id')}`, not `{sid}`")
        return None, None, False, dir_key
    report.add("PASS", "package for us", f"{target} ({sha256_of(body)[:23]}…)")
    if pub is not None:
        try:
            ok = verify(pub, canonical({k: v for k, v in pkg.items() if k != "signature"}),
                        base64.b64decode(pkg.get("signature", "")))
        except (ValueError, RuntimeError, OSError):
            ok = False
        if ok:
            report.add("PASS", "package signature", "verifies under the key you read back on the call")
        else:
            report.add("WARN", "package signature", "does NOT verify under the key you read back on the call",
                       "trust the key you read back, not the directory; tell the counterparty")
    return body, pkg, True, dir_key


def fetch_ca(url, sid: str, theirs: dict, work: str | None, report: "Report", timeout: float = 10.0):
    """Their CA certificate from the directory, checked against their package's
    identity.ca_fingerprint and placed at <work>/theirs-ca.pem if none is there.
    Returns the fingerprint it matched, or None."""
    fp = (theirs.get("identity") or {}).get("ca_fingerprint", "")
    try:
        status, _, body = _get(entry_url(url, sid) + "/ca", timeout, accept="application/pem-certificate-chain")
    except (OSError, urllib.error.URLError) as e:
        report.add("WARN", "CA fetch", f"{e}", "run phase 5 again once the directory answers")
        return None
    if status != 200:
        report.add("WARN", "CA fetch", f"`{sid}` hosts no CA certificate in the directory (HTTP {status})",
                   "run phase 5 again after they run theirs")
        return None
    anchor = anchor_for(body, fp)
    if anchor is None:
        report.add("FAIL", "CA fingerprint", f"no certificate the directory serves for `{sid}` has their package's "
                   f"identity.ca_fingerprint {fp}", "do not use it; ask them which CA is current")
        return None
    report.add("PASS", "CA fingerprint", "the CA certificate the directory serves matches their package's identity.ca_fingerprint")
    if work:
        dest = os.path.join(work, "theirs-ca.pem")
        if os.path.exists(dest):
            if anchor_for(read_bytes(dest), fp) is None:
                report.add("WARN", "theirs-ca.pem", f"{dest} holds no certificate with their ca_fingerprint; left as it is",
                           "the file on disk is what phase 10 places; replace it with the one the voice call confirms")
        else:
            try:
                fd = os.open(dest, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                with os.fdopen(fd, "wb") as f:
                    f.write(anchor)
                report.add("PASS", "theirs-ca.pem", f"placed at {dest}")
            except OSError as e:
                report.add("WARN", "theirs-ca.pem", f"could not place it at {dest}: {e}")
    return fp


def fetch_bundle(url, sid: str, key_b64: str | None, work: str | None, report: "Report", timeout: float = 10.0):
    """Their SPIFFE trust bundle from the directory, checked against the signature of the
    upload that stored it (saltlick SPEC "Checking a hosted file against the key read
    aloud") under their verifying key, and placed at <work>/theirs.spiffe-bundle if none
    is there. Nothing in the package names the bundle, so the signature is the check."""
    resource = f"{sid}/spiffe-bundle"
    try:
        status, headers, body = _get(entry_url(url, sid) + "/spiffe-bundle", timeout)
    except (OSError, urllib.error.URLError) as e:
        report.add("WARN", "SPIFFE bundle fetch", f"{e}", "run phase 5 again once the directory answers")
        return
    if status == 404:
        report.add("UNKNOWN", "SPIFFE bundle", f"`{sid}` hosts none",
                   "needed only when both sides run SPIRE federation (phase 13); run phase 5 again after they upload one")
        return
    if status != 200:
        report.add("WARN", "SPIFFE bundle fetch", f"HTTP {status}", "run phase 5 again")
        return
    writer = headers.get("x-saltlick-key", "")
    sig = headers.get("x-saltlick-signature", "")
    if not key_b64:
        report.add("WARN", "SPIFFE bundle", f"no verifying key for `{sid}` to check it against; not placed",
                   "read their key back on the call, record it, and run phase 5 again")
        return
    try:
        ok = (writer == key_b64 and headers.get("x-saltlick-resource", resource) == resource
              and verify(base64.b64decode(key_b64), request_statement("PUT", resource, body, key_b64), base64.b64decode(sig)))
    except (ValueError, RuntimeError):
        ok = False
    if not ok:
        report.add("FAIL", "SPIFFE bundle signature", f"the bundle the directory serves for `{sid}` is not signed by "
                   f"their verifying key", "do not use it; ask them to run phase 5 again")
        return
    report.add("PASS", "SPIFFE bundle signature", "signed by their verifying key")
    if work:
        dest = os.path.join(work, "theirs.spiffe-bundle")
        if os.path.exists(dest):
            if read_bytes(dest) != body:
                report.add("WARN", "theirs.spiffe-bundle", f"{dest} differs from the bundle `{sid}` serves; left as it is",
                           "the file on disk is what phase 13 imports")
        else:
            try:
                fd = os.open(dest, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                with os.fdopen(fd, "wb") as f:
                    f.write(body)
                report.add("PASS", "theirs.spiffe-bundle", f"placed at {dest}")
            except OSError as e:
                report.add("WARN", "theirs.spiffe-bundle", f"could not place it at {dest}: {e}")


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
    ap.add_argument("--ca")
    ap.add_argument("--theirs")
    ap.add_argument("--their-id")
    ap.add_argument("--keyring")
    ap.add_argument("--work")
    ap.add_argument("--timeout", type=float, default=10.0)
    ap.add_argument("--per-counterparty", action="store_true",
                    default=os.environ.get("FED_DIRECTORY_PER_COUNTERPARTY", "") == "1",
                    help="publish and look up one package per counterparty (several peerings); "
                         "default from FED_DIRECTORY_PER_COUNTERPARTY=1, else off")
    a = ap.parse_args(argv)

    print(f"\n    Directory step: {a.url}")
    print(f"    {ADVISORY}")
    warnings = []
    if not _https(a.url):
        print("    FAIL    the directory URL is not https; saltlick refuses a pointer that is not, so nothing is published")
        return 0

    try:
        ours_raw = read_bytes(a.ours)
        ours = json.loads(ours_raw)
    except (OSError, ValueError) as e:
        print(f"    WARN    our package is not readable ({e}); the directory step is skipped")
        return 0
    # The counterparty a per-counterparty publish is for, read without side effects so
    # that publishing stays the first act, as it is without the flag.
    slot_for = None
    if a.per_counterparty:
        slot_for = a.their_id
        if not slot_for and a.theirs and os.path.isfile(a.theirs):
            try:
                peek = json.loads(read_bytes(a.theirs))
                slot_for = peek.get("sovereign_id") if isinstance(peek, dict) else None
            except (OSError, ValueError):
                slot_for = None
        if slot_for and not SOVEREIGN_ID_RE.match(slot_for):
            slot_for = None
    warnings += publish(a.url, ours_raw, a.package_url, a.key, a.work, a.timeout, a.ca,
                        their_id=slot_for, per_counterparty=a.per_counterparty)

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
        found = None
        if a.per_counterparty:
            found = lookup_slot(a.url, their_id, ours.get("sovereign_id", ""), keyring.get(their_id), a.timeout, checks)
        if found is None:
            found = lookup(a.url, their_id, keyring.get(their_id), a.timeout, checks)
        body, theirs, via_pointer, dir_key = found
        their_fp = fetch_ca(a.url, their_id, theirs, a.work, checks, a.timeout) if theirs is not None else None
        fetch_bundle(a.url, their_id, keyring.get(their_id) or dir_key, a.work, checks, a.timeout)
        checks.print(f"Lookup of `{their_id}`")
        their_key = keyring.get(their_id) or dir_key
        if their_key or their_fp:
            print(f"\n    Read back on the voice call with `{their_id}`:")
            if their_key:
                src = "as you bound it" if keyring.get(their_id) else "as the directory has it; confirm it, then record it in the keyring"
                print(f"      their verifying key ({src}): {key_groups(their_key)}")
            if their_fp:
                print(f"      their CA fingerprint:  {fingerprint_groups(their_fp)}")
        if theirs is not None:
            if a.work:
                try:
                    with open(os.path.join(a.work, "theirs.fetched.json"), "wb") as f:
                        f.write(body)
                except OSError:
                    pass  # a convenience copy for the operator; nothing reads it
            if via_pointer and theirs_on_disk is None and a.theirs:
                # A full package fetched through their pointer: place it where phase 6 reads
                # it. compose still verifies it against the key read back on the call.
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
    print("\n    The out-of-band act that remains: read the verifying key and the CA fingerprint, and\n"
          "    after phase 6 the agreement digest, aloud to the counterparty. The directory replaces none of them.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except SystemExit:
        raise
    except Exception as e:  # noqa: BLE001 — the directory step never fails a peering
        print(f"    WARN    directory step error: {e}; carrying on without it")
        sys.exit(0)
