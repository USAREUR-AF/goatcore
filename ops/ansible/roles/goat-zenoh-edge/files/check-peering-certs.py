#!/usr/bin/env python3
"""Check the peering edge's certificate material before the role installs it.

Runs on the controller, where phase 8 (mint-peering-leaves.sh) left the leaves
and phase 5 (directory.py, or the hand exchange) left the partner's CA. It reads
and writes nothing on the edge: the role installs the files only when this exits 0.

Checks, all of them, reporting every failure rather than the first:
  - each leaf and key exists and is a regular file; each leaf file holds exactly
    one certificate; each key has mode 0600;
  - each key belongs to its certificate (public keys compared as DER);
  - each leaf chains to OUR CA as the agreement names it (the certificate in
    --our-ca whose SHA-256 is our party's ca_fingerprint), for TLS client and
    server use, with default trust locations disabled, and is not expired;
  - the partner CA file holds exactly one certificate, and its SHA-256 over DER
    equals the ca_fingerprint the agreement names for the partner. A second
    certificate in that file would be trusted on the partner-facing listener
    without the agreement naming it, so it is refused, not ignored.

Usage:
  check-peering-certs.py --src-dir <pki> --partner-ca <pem> --agreement <json>
                         --partner-id <id> --our-ca <pem|-> [--partner-fingerprint sha256:<hex>]
                         [--min-valid-seconds N]

Exit 0, one `ok:` line per check and a `checksums:` line (JSON, SHA-1 of each
source file as checked), or exit 1 and one `refused:` line per failure.
"""
import argparse
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
import tempfile

LEAVES = ("peering-outward", "peering-inward")
FINGERPRINT = re.compile(r"^sha256:[0-9a-f]{64}$")
PEM_CERT = re.compile(rb"-----BEGIN CERTIFICATE-----\s.*?-----END CERTIFICATE-----", re.S)


def openssl(*args, data=None):
    result = subprocess.run(["openssl", *map(str, args)], input=data, capture_output=True,
                            env={**os.environ, "LC_ALL": "C"})
    return result.returncode, result.stdout, result.stderr.decode(errors="replace").strip()


def der_of(pem):
    rc, out, err = openssl("x509", "-outform", "DER", data=pem)
    if rc:
        raise ValueError("not a readable certificate: " + (err.splitlines() or ["openssl failed"])[-1])
    return out


def sha256(der):
    return "sha256:" + hashlib.sha256(der).hexdigest()


def public_key_der(path, certificate):
    if certificate:
        rc, pem, err = openssl("x509", "-in", path, "-pubkey", "-noout")
        if rc:
            return None, err
        rc, der, err = openssl("pkey", "-pubin", "-outform", "DER", data=pem)
    else:
        rc, der, err = openssl("pkey", "-in", path, "-passin", "pass:", "-pubout", "-outform", "DER")
    return (None, err) if rc else (der, "")


def agreement_fingerprints(path, partner):
    with open(path, encoding="utf-8") as f:
        parties = json.load(f).get("parties") or []
    ids = [p.get("sovereign_id") for p in parties]
    if len(parties) != 2 or partner not in ids:
        raise ValueError(f"{path} does not name '{partner}' as one of exactly two parties (it names {ids})")
    theirs = next(p for p in parties if p.get("sovereign_id") == partner).get("ca_fingerprint", "")
    ours = next(p for p in parties if p.get("sovereign_id") != partner).get("ca_fingerprint", "")
    return ours, theirs


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--src-dir", required=True)
    ap.add_argument("--partner-ca", required=True)
    ap.add_argument("--agreement", required=True)
    ap.add_argument("--partner-id", required=True)
    ap.add_argument("--our-ca", required=True, help="our CA as PEM, '-' for stdin")
    ap.add_argument("--partner-fingerprint", default="")
    ap.add_argument("--min-valid-seconds", type=int, default=0)
    a = ap.parse_args()

    ok, refused = [], []

    try:
        ours_fp, theirs_fp = agreement_fingerprints(a.agreement, a.partner_id)
    except (OSError, ValueError) as e:
        print(f"refused: cannot read the agreement: {e}. It is phase 6's output ({a.agreement}).")
        return 1
    if a.partner_fingerprint:
        if a.partner_fingerprint != theirs_fp:
            refused.append(f"goat_zenoh_edge_partner_ca_fingerprint is {a.partner_fingerprint}, but the agreement "
                           f"names {theirs_fp} for '{a.partner_id}'. Remove the setting or correct it; the "
                           "agreement is the record both parties signed.")
        theirs_fp = a.partner_fingerprint
    for who, fp in (("our", ours_fp), (f"'{a.partner_id}'s", theirs_fp)):
        if not FINGERPRINT.match(fp or ""):
            refused.append(f"the agreement's {who} ca_fingerprint is not sha256:<64 lowercase hex>: {fp!r}")
    if refused:
        print("\n".join("refused: " + r for r in refused))
        return 1

    # --- the partner CA: exactly one certificate, the one the agreement names ---
    try:
        with open(a.partner_ca, "rb") as f:
            partner_pem = f.read()
        certs = PEM_CERT.findall(partner_pem)
        if len(certs) != 1:
            refused.append(f"{a.partner_ca} holds {len(certs)} certificates; it must hold exactly one, the CA "
                           f"whose fingerprint the agreement names ({theirs_fp}). Every certificate in it would "
                           "be trusted on the partner-facing listener.")
        else:
            got = sha256(der_of(certs[0]))
            if got != theirs_fp:
                refused.append(f"{a.partner_ca} has fingerprint {got}, but the agreement names {theirs_fp} for "
                               f"'{a.partner_id}'. Do not install it: ask the partner which CA is current, "
                               "and fetch it again (phase 5).")
            else:
                ok.append(f"partner CA {a.partner_ca} matches the agreement's fingerprint {theirs_fp}")
    except (OSError, ValueError) as e:
        refused.append(f"partner CA {a.partner_ca}: {e}. It is phase 5's output (theirs-ca.pem).")

    # --- our CA: the certificate the agreement names for us ----------------------
    our_anchor = None
    try:
        our_pem = sys.stdin.buffer.read() if a.our_ca == "-" else open(a.our_ca, "rb").read()
        for c in PEM_CERT.findall(our_pem):
            if sha256(der_of(c)) == ours_fp:
                our_anchor = c
                break
        if our_anchor is None:
            refused.append(f"our CA ({'goat_zenoh_edge_our_ca_path, read from the edge' if a.our_ca == '-' else a.our_ca}) "
                           f"holds no certificate with the fingerprint the agreement names for us ({ours_fp}); "
                           "the leaves cannot be checked against the CA the partner was told about.")
    except (OSError, ValueError) as e:
        refused.append(f"our CA: {e}")

    # --- the leaves -------------------------------------------------------------
    trust = ["-no-CAfile", "-no-CApath"]
    _, h_out, h_err = openssl("verify", "-help")
    if b"-no-CAstore" in h_out or "-no-CAstore" in h_err:
        trust.append("-no-CAstore")
    with tempfile.TemporaryDirectory(prefix="edge-cert-check-") as tmp:
        anchor_path = os.path.join(tmp, "our-ca.pem")
        if our_anchor is not None:
            with open(anchor_path, "wb") as f:
                f.write(our_anchor + b"\n")
        for name in LEAVES:
            cert = os.path.join(a.src_dir, name + ".pem")
            key = os.path.join(a.src_dir, name + "-key.pem")
            missing = [p for p in (cert, key) if not os.path.isfile(p)]
            if missing:
                refused.append(f"{name}: no file at {', '.join(missing)}. Run phase 8 (mint-peering-leaves.sh).")
                continue
            with open(cert, "rb") as f:
                blocks = len(PEM_CERT.findall(f.read()))
            if blocks != 1:
                refused.append(f"{name}: {cert} holds {blocks} certificates; it must hold exactly the one leaf "
                               "phase 8 minted, because every certificate in it is presented to the peer.")
                continue
            mode = stat.S_IMODE(os.stat(key).st_mode)
            if mode != 0o600:
                refused.append(f"{name}: {key} has mode {mode:04o}; a private key must be 0600.")
            cpub, cerr = public_key_der(cert, True)
            kpub, kerr = public_key_der(key, False)
            if cpub is None or kpub is None:
                refused.append(f"{name}: unreadable {'certificate ' + cert if cpub is None else 'key ' + key}: "
                               f"{(cerr or kerr).splitlines()[-1] if (cerr or kerr) else 'openssl failed'}")
                continue
            if cpub != kpub:
                refused.append(f"{name}: {key} does not belong to {cert} (their public keys differ). "
                               "Re-run phase 8 for this leaf; do not pair files by hand.")
            else:
                ok.append(f"{name}: key matches certificate")
            if our_anchor is None:
                continue
            chained = True
            for purpose in ("sslserver", "sslclient"):
                # -partial_chain: our named CA may be an intermediate under a shared
                # root; with default locations disabled it is still the only anchor.
                rc, out, err = openssl("verify", *trust, "-partial_chain", "-trusted", anchor_path,
                                       "-purpose", purpose, cert)
                if rc:
                    reason = (err or out.decode(errors="replace")).strip().splitlines()
                    refused.append(f"{name}: {cert} does not chain to our CA {ours_fp} for {purpose}: "
                                   f"{reason[-1] if reason else 'verify failed'}")
                    chained = False
                    break
            if chained and a.min_valid_seconds > 0:
                rc, _, _ = openssl("x509", "-in", cert, "-noout", "-checkend", a.min_valid_seconds)
                if rc:
                    refused.append(f"{name}: {cert} expires within {a.min_valid_seconds} seconds. "
                                   "Re-run phase 8 with --rotate.")
                    chained = False
            if chained:
                ok.append(f"{name}: chains to our CA {ours_fp}")

    # SHA-1 of each source as checked, for the copy module's `checksum`: the role
    # installs exactly these bytes, or fails if a file changed after the check.
    sums = {}
    for path in [os.path.join(a.src_dir, n + s) for n in LEAVES for s in (".pem", "-key.pem")] + [a.partner_ca]:
        try:
            with open(path, "rb") as f:
                sums[path] = hashlib.sha1(f.read()).hexdigest()
        except OSError:
            pass
    for line in ok:
        print("ok: " + line)
    if not refused:
        print("checksums: " + json.dumps(sums, sort_keys=True))
    for line in refused:
        print("refused: " + line)
    return 1 if refused else 0


if __name__ == "__main__":
    sys.exit(main())
