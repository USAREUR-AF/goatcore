#!/usr/bin/env python3
"""A test double of the acceptance-result signer and verifier, for the tests of
acceptance.sh on the in-process fabric only.

In a real run the signer and verifier are goatbuilder's own, which call the one
implementation in crates/goat-federation/src/acceptance_sig.rs. This double
implements the same rule (docs/design/federation-acceptance.md §6.1) so the
runner's tests need no Rust build, and test_acceptance_sig_double.py holds it to
the Rust implementation's committed vector byte for byte. It reads its keys from
the environment, never from a deployment:

  FED_TEST_SIG_SEED   the signer's Ed25519 seed file (64 hex characters)
  FED_TEST_SIG_KEYS   the bound public keys, `<sid>=<44 base64>,<sid>=<44 base64>`

  acceptance_sig_double.py sign   --agreement-sha256 H --session S --attempt N --by US --for THEM
                                  --echoes C --result FILE          (prints the signature document)
  acceptance_sig_double.py verify --agreement-sha256 H --session S --attempt N --us US --them THEM
                                  --our-challenge C --result FILE --signature FILE
                                  (prints {key_sha256, key_b64, result_sha256}; exit 2 and
                                   `GB-272: ...` or `GB-273: ...` on stderr when refused)
"""
import argparse
import base64
import hashlib
import json
import os
import re
import sys

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey, Ed25519PublicKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

RESULT_DOMAIN = "goat-acceptance-result/v1\n"
SCHEMA = "goat-acceptance-signature/v1"
HEX64 = re.compile(r"[0-9a-f]{64}")
CHALLENGE = re.compile(r"[0-9a-f]{16,64}")
SOVEREIGN = re.compile(r"[a-z0-9][a-z0-9-]{1,15}")
SESSION_ID = re.compile(r"ses-[0-9A-Za-z-]{1,64}")


class Refused(Exception):
    def __init__(self, code, detail):
        super().__init__("%s: %s" % (code, detail))


def statement(f, result_sha, key_sha):
    return ("%sagreement sha256:%s\nsession %s\nattempt %d\nby %s\nfor %s\nechoes %s\n"
            "result sha256:%s\nkey sha256:%s\n" % (RESULT_DOMAIN, f["agreement_sha256"], f["session_id"],
                                                   f["attempt"], f["by"], f["for"], f["echoes"], result_sha,
                                                   key_sha))


def check_fields(f):
    ok = (HEX64.fullmatch(f["agreement_sha256"]) and SESSION_ID.fullmatch(f["session_id"])
          and isinstance(f["attempt"], int) and not isinstance(f["attempt"], bool) and f["attempt"] >= 1
          and SOVEREIGN.fullmatch(f["by"]) and SOVEREIGN.fullmatch(f["for"]) and CHALLENGE.fullmatch(f["echoes"]))
    if not ok:
        raise Refused("GB-272", "a field of the signature document is not of its shape")


def canonical(v):
    return json.dumps(v, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def sign(seed, f, result):
    check_fields(f)
    sk = Ed25519PrivateKey.from_private_bytes(seed)
    key_sha = hashlib.sha256(sk.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)).hexdigest()
    result_sha = hashlib.sha256(result).hexdigest()
    sig = sk.sign(statement(f, result_sha, key_sha).encode())
    return canonical(dict(f, schema=SCHEMA, result_sha256=result_sha, key_sha256=key_sha,
                          signature=base64.b64encode(sig).decode()))


def verify(result, doc_bytes, keys, expect):
    try:
        doc = json.loads(doc_bytes)
    except ValueError as e:
        raise Refused("GB-272", "the signature document is not readable: %s" % e)
    if not isinstance(doc, dict) or doc.get("schema") != SCHEMA:
        raise Refused("GB-272", "the signature document is not %s" % SCHEMA)
    try:
        f = {k: doc[k] for k in ("agreement_sha256", "session_id", "attempt", "by", "for", "echoes")}
        claimed_result, claimed_key, sig = doc["result_sha256"], doc["key_sha256"], doc["signature"]
    except KeyError as e:
        raise Refused("GB-272", "the signature document has no %s" % e)
    check_fields(f)
    other = []
    for name, want in (("agreement_sha256", expect["agreement_sha256"]), ("session_id", expect["session_id"]),
                       ("attempt", expect["attempt"]), ("by", expect["them"]), ("for", expect["us"])):
        if f[name] != want:
            other.append("%s %r is not ours (%r)" % (name, f[name], want))
    if f["echoes"] != expect["our_challenge"]:
        other.append("it does not echo the challenge issued to us, so it is not from this session's samples")
    if other:
        raise Refused("GB-273", "; ".join(other))
    bound = keys.get(expect["them"])
    if bound is None:
        raise Refused("GB-272", "no key is bound to %s" % expect["them"])
    bound_sha = hashlib.sha256(bound).hexdigest()
    if claimed_key != bound_sha:
        raise Refused("GB-272", "it names key sha256:%s, and the key bound to %s is sha256:%s"
                      % (claimed_key, expect["them"], bound_sha))
    digest = hashlib.sha256(result).hexdigest()
    if claimed_result != digest:
        raise Refused("GB-272", "it signs result sha256:%s, and the result read is sha256:%s"
                      % (claimed_result, digest))
    try:
        Ed25519PublicKey.from_public_bytes(bound).verify(base64.b64decode(sig, validate=True),
                                                         statement(f, digest, bound_sha).encode())
    except (InvalidSignature, ValueError):
        raise Refused("GB-272", "the signature does not verify under the key bound to %s" % expect["them"])
    return {"key_sha256": bound_sha, "key_b64": base64.b64encode(bound).decode(), "result_sha256": digest}


def main(argv=None):
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("sign", "verify"):
        p = sub.add_parser(name)
        p.add_argument("--agreement-sha256", required=True)
        p.add_argument("--session", required=True)
        p.add_argument("--attempt", type=int, required=True)
        p.add_argument("--result", required=True)
        if name == "sign":
            p.add_argument("--by", required=True)
            p.add_argument("--for", dest="for_", required=True)
            p.add_argument("--echoes", required=True)
        else:
            p.add_argument("--us", required=True)
            p.add_argument("--them", required=True)
            p.add_argument("--our-challenge", required=True)
            p.add_argument("--signature", required=True)
    a = ap.parse_args(argv)
    with open(a.result, "rb") as fh:
        result = fh.read()
    try:
        if a.cmd == "sign":
            with open(os.environ["FED_TEST_SIG_SEED"]) as fh:
                seed = bytes.fromhex(fh.read().strip())
            f = {"agreement_sha256": a.agreement_sha256, "session_id": a.session, "attempt": a.attempt,
                 "by": a.by, "for": a.for_, "echoes": a.echoes}
            if os.environ.get("FED_TEST_SIG_TAMPER"):
                result += b" "  # signs other bytes than the result published: the verifier must refuse it
            print(sign(seed, f, result))
            return 0
        keys = {}
        for item in filter(None, os.environ.get("FED_TEST_SIG_KEYS", "").split(",")):
            sid, b64 = item.split("=", 1)
            keys[sid] = base64.b64decode(b64 + "=" * (-len(b64) % 4))
        with open(a.signature, "rb") as fh:
            doc = fh.read()
        expect = {"agreement_sha256": a.agreement_sha256, "session_id": a.session, "attempt": a.attempt,
                  "us": a.us, "them": a.them, "our_challenge": a.our_challenge}
        print(json.dumps(verify(result, doc, keys, expect), sort_keys=True))
        return 0
    except Refused as e:
        print(str(e), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
