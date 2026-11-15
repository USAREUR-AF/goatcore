#!/usr/bin/env python3
"""generate.py: the far-term federation agreement test fixture, made or checked.

`goat federate verify` and `apply` judge an agreement's term on today's date,
as in production, so a test that runs them through a binary needs an
agreement whose term has not ended on the day it runs. The normative
conformance vectors (ops/federation/vectors/) are judged on their now.txt and
are never changed for this. This fixture is that agreement: two packages
between the test parties `xta` and `xtb` (the user-assigned range every
example and fixture uses, spec §3, and not the vectors' `xaa`/`xab`), signed
with test keys made here from fixed names, both expiring 2099-12-31, composed
by the binary given.

Everything is deterministic: the seeds are the ASCII names below padded to
32 bytes with NUL, Ed25519 signatures are deterministic, and composition is
evaluated on ISSUED. So --check makes the fixture again in a scratch
directory and compares it byte for byte with the committed one.

The keys sign this fixture and nothing else. Their seeds are the names above,
so anyone can sign with them and a package signed with one proves nothing
about who made it; never bind one to a party. Only the public halves are
written out (keys/), the ones compose is given.

usage:
  generate.py <goat or goatbuilder binary>            write the fixture here
  generate.py --check <goat or goatbuilder binary>    exit 1 unless it matches,
                                                      or within a year of EXPIRES
"""

import base64
import datetime
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

HERE = os.path.dirname(os.path.abspath(__file__))
ISSUED = "2026-11-15"
EXPIRES = "2099-12-31"
PARTIES = ("xta", "xtb")
RENEW_DAYS = 365  # --check fails this many days before EXPIRES
SEED_NAME = "goat-federation-test-fixture-%s"


def seed_of(sid):
    name = (SEED_NAME % sid).encode("ascii")
    assert len(name) <= 32
    return name + b"\0" * (32 - len(name))


def canonical(value):
    """The package format's canonical form for this fixture's values: members
    sorted, no whitespace, integers only, ASCII strings (§4). Every value here
    is ASCII, where code-unit order and code-point order agree."""
    text = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    assert text.isascii()
    return text.encode("ascii")


def stand_in(label, n=32):
    """A fixed value of n bytes, so the fixture carries no real fingerprint or key."""
    return hashlib.sha256(("goat-federation-test-fixture " + label).encode()).digest()[:n]


def package(sid, listener, subnet, mtu, release, accept):
    return {
        "spec": "federation-package/v1",
        "sovereign_id": sid,
        "issued": ISSUED,
        "expires": EXPIRES,
        "identity": {
            "trust_domain": "spiffe://%s.example" % sid,
            "ca_fingerprint": "sha256:" + stand_in(sid + " ca").hex(),
        },
        "capabilities": {
            "fabric_fips_image": True,
            "fabric_version": "1.9.0",
            "pqc_capable": True,
            "transports": ["ipsec-fips", "wireguard"],
        },
        "transport_offer": {
            "endpoint": "_fed._udp.%s.example" % sid,
            "listener": listener,
            "measured_mtu": mtu,
            "proposed_link_subnet": subnet,
            "public_key": base64.b64encode(stand_in(sid + " transport")).decode(),
        },
        "data_offer": dict(accept, release=release),
        "cut": {"contact": "fed-ops@%s.example" % sid},
    }


def make(binary, out):
    os.makedirs(os.path.join(out, "keys"), exist_ok=True)
    pkgs = [
        package(PARTIES[0], True, "169.254.30.0/31", 1380, ["release/weather/**"],
                {"accept_hz": 100, "accept_bytes_per_message": 1048576, "accept_distinct_keys": 5000}),
        package(PARTIES[1], False, "169.254.40.0/31", 1420, ["release/imagery/**"],
                {"accept_hz": 50, "accept_bytes_per_message": 262144, "accept_distinct_keys": 2000}),
    ]
    keyflags = []
    paths = []
    for p in pkgs:
        sid = p["sovereign_id"]
        key = Ed25519PrivateKey.from_private_bytes(seed_of(sid))
        pub = base64.b64encode(key.public_key().public_bytes_raw()).decode()
        with open(os.path.join(out, "keys", sid + ".public.b64"), "w") as fh:
            fh.write(pub + "\n")
        p["signature"] = base64.b64encode(key.sign(canonical(p))).decode()
        path = os.path.join(out, "package-%s.canonical.json" % sid)
        with open(path, "wb") as fh:
            fh.write(canonical(p))
        paths.append(path)
        keyflags += ["--key", "%s=%s" % (sid, pub)]
    agreement = os.path.join(out, "agreement.canonical.json")
    r = subprocess.run([binary, "federate", "compose", paths[0], paths[1], *keyflags,
                        "--now", ISSUED, "--out", agreement], capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("generate.py: compose refused (exit %d):\n%s%s" % (r.returncode, r.stdout, r.stderr))
    with open(agreement, "rb") as fh:
        raw = fh.read()
    with open(os.path.join(out, "agreement.digest.txt"), "w") as fh:
        fh.write("sha256:" + hashlib.sha256(raw).hexdigest() + "\n")


OUTPUTS = ("agreement.canonical.json", "agreement.digest.txt",
           "package-xta.canonical.json", "package-xtb.canonical.json",
           "keys/xta.public.b64", "keys/xtb.public.b64")


def main(argv):
    check = argv[:1] == ["--check"]
    rest = argv[1:] if check else argv
    if len(rest) != 1:
        sys.exit(__doc__.split("usage:")[1])
    binary = os.path.abspath(rest[0])
    today = datetime.datetime.now(datetime.timezone.utc).date()
    left = (datetime.date.fromisoformat(EXPIRES) - today).days
    if check and left < RENEW_DAYS:
        print("generate.py: FAIL the far-term fixture expires %s, %d days from %s; raise EXPIRES "
              "and regenerate %d or more days ahead" % (EXPIRES, left, today, RENEW_DAYS))
        return 1
    if not check:
        make(binary, HERE)
        print("generate.py: wrote %d files under %s" % (len(OUTPUTS), HERE))
        return 0
    scratch = tempfile.mkdtemp(prefix="far-term-")
    try:
        make(binary, scratch)
        bad = []
        for rel in OUTPUTS:
            try:
                with open(os.path.join(HERE, rel), "rb") as fh:
                    committed = fh.read()
            except OSError:
                bad.append("%s is missing" % rel)
                continue
            with open(os.path.join(scratch, rel), "rb") as fh:
                if fh.read() != committed:
                    bad.append("%s differs from what generate.py makes" % rel)
    finally:
        shutil.rmtree(scratch)
    for b in bad:
        print("generate.py: FAIL " + b)
    if bad:
        print("generate.py: regenerate with `python3 %s <binary>` and review the diff"
              % os.path.relpath(__file__))
        return 1
    print("generate.py: ok, the far-term fixture is what generate.py makes")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
