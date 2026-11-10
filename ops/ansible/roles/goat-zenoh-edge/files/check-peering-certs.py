#!/usr/bin/env python3
"""Check the peering edge's certificate material before the role installs it.

Runs on the controller, where phase 8 (mint-peering-leaves.sh) left the leaves
and phase 5 (directory.py, or the hand exchange) left the partner's CA. It reads
and writes nothing on the edge: the role installs the files only when this exits 0.

TRUST ASSUMPTION. The agreement file on the controller is taken as the verified
agreement: phase 6 verified its signatures, and this script does not verify them
again. Whoever can write the controller's working directory can change what is
checked here.

Each file is read once (no symlink, regular files only) into a private temporary
directory; every check runs on those copies, and the checksums printed are of
those same bytes, so the role installs exactly what was checked.

Checks, all of them, reporting every failure rather than the first:
  - the agreement names exactly two distinct parties, one of them the partner;
  - each leaf file holds exactly one certificate; each key has mode 0600 and
    belongs to its certificate (public keys compared as DER);
  - each leaf carries the identity the agreement derives, as mint-peering-leaves.sh
    does: outward CN fed.<us>.<them>.01, inward CN fed.<them>.<us>.01, an exact
    subject, a DNS SAN equal to that CN, and basicConstraints CA:FALSE; neither
    leaf is our CA or the partner CA itself;
  - each leaf chains to OUR CA as the agreement names it (the certificate in
    --our-ca whose SHA-256 is our party's ca_fingerprint), for TLS client and
    server use, with default trust locations disabled, and is valid for at least
    --min-valid-seconds;
  - the partner CA file holds exactly one certificate, a CA (CA:TRUE) valid for
    at least --min-valid-seconds, whose SHA-256 over DER equals the ca_fingerprint
    the agreement names for the partner. A second certificate in that file would
    be trusted on the partner-facing listener without the agreement naming it,
    so it is refused, not ignored.

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
SOVEREIGN_ID = re.compile(r"^[A-Za-z0-9_-]+$")
PEM_CERT = re.compile(rb"-----BEGIN CERTIFICATE-----\s.*?-----END CERTIFICATE-----", re.S)
MAX_BYTES = 1 << 20


class Refused(Exception):
    pass


def openssl(*args, data=None):
    result = subprocess.run(["openssl", *map(str, args)], input=data, capture_output=True,
                            env={**os.environ, "LC_ALL": "C"})
    return result.returncode, result.stdout, result.stderr.decode(errors="replace").strip()


def last_line(text, default="openssl failed"):
    lines = [ln for ln in (text or "").splitlines() if ln.strip()]
    return lines[-1] if lines else default


def read_once(path):
    """The file's bytes and mode, read through one descriptor: no symlink, regular file only."""
    try:
        fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0))
    except OSError as e:
        raise Refused(f"cannot open {path}: {e.strerror}" + (" (a symlink is refused)" if os.path.islink(path) else ""))
    try:
        st = os.fstat(fd)
        if not stat.S_ISREG(st.st_mode):
            raise Refused(f"{path} is not a regular file")
        data = b""
        while len(data) <= MAX_BYTES:
            chunk = os.read(fd, 65536)
            if not chunk:
                break
            data += chunk
        if len(data) > MAX_BYTES:
            raise Refused(f"{path} is larger than {MAX_BYTES} bytes")
        return data, stat.S_IMODE(st.st_mode)
    finally:
        os.close(fd)


def der_of(pem):
    rc, out, err = openssl("x509", "-outform", "DER", data=pem)
    if rc:
        raise Refused("not a readable certificate: " + last_line(err))
    return out


def sha256(der):
    return "sha256:" + hashlib.sha256(der).hexdigest()


def text_of(path):
    rc, out, err = openssl("x509", "-in", path, "-noout", "-text")
    if rc:
        raise Refused(f"{path}: not a readable certificate: {last_line(err)}")
    return out.decode(errors="replace")


def extension(text, name):
    m = re.search(r"X509v3 " + re.escape(name) + r":[^\n]*\n\s*([^\n]+)", text)
    return m[1].strip() if m else None


def public_key_der(path, certificate):
    if certificate:
        rc, pem, err = openssl("x509", "-in", path, "-pubkey", "-noout")
        if rc:
            return None, err
        rc, der, err = openssl("pkey", "-pubin", "-outform", "DER", data=pem)
    else:
        rc, der, err = openssl("pkey", "-in", path, "-passin", "pass:", "-pubout", "-outform", "DER")
    return (None, err) if rc else (der, "")


def read_agreement(raw, path, partner):
    """(us, ours_fp, theirs_fp) from the agreement, or Refused naming what is wrong."""
    try:
        ag = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, ValueError) as e:
        raise Refused(f"{path} is not JSON: {e}")
    parties = ag.get("parties") if isinstance(ag, dict) else None
    if not isinstance(parties, list) or len(parties) != 2 or not all(isinstance(p, dict) for p in parties):
        raise Refused(f"{path} must carry `parties` as a list of exactly two objects")
    ids = [p.get("sovereign_id") for p in parties]
    if not all(isinstance(i, str) and SOVEREIGN_ID.match(i) for i in ids):
        raise Refused(f"{path}: every party needs a sovereign_id of letters, digits, '-' or '_' (it names {ids})")
    if ids[0] == ids[1]:
        raise Refused(f"{path} names the same sovereign_id twice ({ids[0]})")
    if partner not in ids:
        raise Refused(f"{path} does not name '{partner}' as a party (it names {ids})")
    them = parties[ids.index(partner)]
    ours = parties[1 - ids.index(partner)]
    return ours["sovereign_id"], ours.get("ca_fingerprint", ""), them.get("ca_fingerprint", "")


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
    sums = {}

    def emit():
        for line in ok:
            print("ok: " + line)
        for line in refused:
            print("refused: " + line)
        if not refused:
            print("checksums: " + json.dumps(sums, sort_keys=True))
        return 1 if refused else 0

    # Read our CA from stdin before anything can return early: a writer whose
    # reader has exited gets a broken pipe, and Ansible then loses this output.
    stdin_ca = sys.stdin.buffer.read(MAX_BYTES + 1) if a.our_ca == "-" else None

    if a.min_valid_seconds < 0:
        refused.append("--min-valid-seconds must not be negative")
        return emit()

    # --- the agreement -------------------------------------------------------------
    try:
        raw, _ = read_once(a.agreement)
        us, ours_fp, theirs_fp = read_agreement(raw, a.agreement, a.partner_id)
    except Refused as e:
        refused.append(f"cannot use the agreement: {e}. It is phase 6's output ({a.agreement}).")
        return emit()
    if a.partner_fingerprint:
        if a.partner_fingerprint != theirs_fp:
            refused.append(f"goat_zenoh_edge_partner_ca_fingerprint is {a.partner_fingerprint}, but the agreement "
                           f"names {theirs_fp} for '{a.partner_id}'. Remove the setting or correct it; the "
                           "agreement is the record both parties signed.")
    for who, fp in (("our", ours_fp), (f"'{a.partner_id}'s", theirs_fp)):
        if not isinstance(fp, str) or not FINGERPRINT.match(fp):
            refused.append(f"the agreement's {who} ca_fingerprint is not sha256:<64 lowercase hex>: {fp!r}")
    if refused:
        return emit()
    expected_cn = {"peering-outward": f"fed.{us}.{a.partner_id}.01",
                   "peering-inward": f"fed.{a.partner_id}.{us}.01"}

    with tempfile.TemporaryDirectory(prefix="edge-cert-check-") as tmp:
        os.chmod(tmp, 0o700)

        def stage(src, name):
            data, mode = read_once(src)
            dst = os.path.join(tmp, name)
            with open(dst, "wb") as f:
                f.write(data)
            sums[src] = hashlib.sha1(data).hexdigest()
            return dst, data, mode

        # --- the partner CA: one CA certificate, the one the agreement names -------
        partner_der = None
        try:
            pca, partner_pem, _ = stage(a.partner_ca, "partner-ca.pem")
            certs = PEM_CERT.findall(partner_pem)
            if len(certs) != 1:
                raise Refused(f"holds {len(certs)} certificates; it must hold exactly one, the CA whose "
                              f"fingerprint the agreement names ({theirs_fp}). Every certificate in it would be "
                              "trusted on the partner-facing listener.")
            partner_der = der_of(certs[0])
            got = sha256(partner_der)
            if got != theirs_fp:
                raise Refused(f"has fingerprint {got}, but the agreement names {theirs_fp} for "
                              f"'{a.partner_id}'. Do not install it: ask the partner which CA is current, and "
                              "fetch it again (phase 5).")
            bc = extension(text_of(pca), "Basic Constraints") or ""
            if not bc.startswith("CA:TRUE"):
                raise Refused(f"is not a CA certificate (basicConstraints {bc or 'absent'}).")
            rc, _, _ = openssl("x509", "-in", pca, "-noout", "-checkend", a.min_valid_seconds)
            if rc:
                raise Refused(f"expires within {a.min_valid_seconds} seconds (or has expired). Ask the partner "
                              "for their current CA; the agreement names its fingerprint.")
            ok.append(f"partner CA {a.partner_ca} is a CA matching the agreement's fingerprint {theirs_fp}")
        except Refused as e:
            refused.append(f"partner CA {a.partner_ca} {e}" if str(e).startswith(("holds", "has", "is", "expires"))
                           else f"partner CA: {e}. It is phase 5's output (theirs-ca.pem).")

        # --- our CA: the certificate the agreement names for us --------------------
        our_der = None
        anchor_path = os.path.join(tmp, "our-ca.pem")
        try:
            if stdin_ca is not None and len(stdin_ca) > MAX_BYTES:
                raise Refused(f"our CA on stdin is larger than {MAX_BYTES} bytes")
            our_pem = stdin_ca if stdin_ca is not None else read_once(a.our_ca)[0]
            for c in PEM_CERT.findall(our_pem):
                if sha256(der_of(c)) == ours_fp:
                    our_der = der_of(c)
                    with open(anchor_path, "wb") as f:
                        f.write(c + b"\n")
                    break
            if our_der is None:
                where = "goat_zenoh_edge_our_ca_path, read from the edge" if a.our_ca == "-" else a.our_ca
                refused.append(f"our CA ({where}) holds no certificate with the fingerprint the agreement names "
                               f"for us ({ours_fp}); the leaves cannot be checked against the CA the partner "
                               "was told about.")
        except Refused as e:
            refused.append(f"our CA: {e}")

        trust = ["-no-CAfile", "-no-CApath"]
        _, h_out, h_err = openssl("verify", "-help")
        if b"-no-CAstore" in h_out or "-no-CAstore" in h_err:
            trust.append("-no-CAstore")

        # --- the leaves -------------------------------------------------------------
        for name in LEAVES:
            src_cert = os.path.join(a.src_dir, name + ".pem")
            src_key = os.path.join(a.src_dir, name + "-key.pem")
            try:
                cert, cert_pem, _ = stage(src_cert, name + ".pem")
                key, _, key_mode = stage(src_key, name + "-key.pem")
            except Refused as e:
                refused.append(f"{name}: {e}. Run phase 8 (mint-peering-leaves.sh).")
                continue
            failed = len(refused)
            blocks = PEM_CERT.findall(cert_pem)
            if len(blocks) != 1:
                refused.append(f"{name}: {src_cert} holds {len(blocks)} certificates; it must hold exactly the one "
                               "leaf phase 8 minted, because every certificate in it is presented to the peer.")
                continue
            if key_mode != 0o600:
                refused.append(f"{name}: {src_key} has mode {key_mode:04o}; a private key must be 0600.")
            try:
                leaf_der = der_of(blocks[0])
                text = text_of(cert)
            except Refused as e:
                refused.append(f"{name}: {src_cert}: {e}")
                continue
            if leaf_der in (our_der, partner_der):
                refused.append(f"{name}: {src_cert} is a CA certificate itself, not a leaf minted under it.")
                continue
            cpub, cerr = public_key_der(cert, True)
            kpub, kerr = public_key_der(key, False)
            if cpub is None or kpub is None:
                refused.append(f"{name}: unreadable {'certificate ' + src_cert if cpub is None else 'key ' + src_key}: "
                               f"{last_line(cerr or kerr)}")
                continue
            if cpub != kpub:
                refused.append(f"{name}: {src_key} does not belong to {src_cert} (their public keys differ). "
                               "Re-run phase 8 for this leaf; do not pair files by hand.")

            # The identity the agreement derives, as mint-peering-leaves.sh mints it.
            want = expected_cn[name]
            rc, subj, _ = openssl("x509", "-in", cert, "-noout", "-subject", "-nameopt", "RFC2253")
            subject = subj.decode(errors="replace").strip().partition("=")[2].strip() if rc == 0 else ""
            if subject != "CN=" + want:
                refused.append(f"{name}: {src_cert} has subject '{subject}', but the agreement derives "
                               f"'CN={want}'. It was minted for another peering or by hand; re-run phase 8.")
            san = extension(text, "Subject Alternative Name") or ""
            dns = {s.strip()[4:].lower() for s in san.split(",") if s.strip().startswith("DNS:")}
            if want.lower() not in dns:
                refused.append(f"{name}: {src_cert} has no DNS SAN '{want}' (SANs: {san or 'none'}).")
            bc = extension(text, "Basic Constraints")
            if bc != "CA:FALSE":
                refused.append(f"{name}: {src_cert} must carry basicConstraints CA:FALSE (it has {bc or 'none'}).")

            if our_der is not None:
                for purpose in ("sslserver", "sslclient"):
                    # -partial_chain: our named CA may be an intermediate under a shared
                    # root; with default locations disabled it is still the only anchor.
                    rc, out, err = openssl("verify", *trust, "-partial_chain", "-trusted", anchor_path,
                                           "-purpose", purpose, cert)
                    if rc:
                        refused.append(f"{name}: {src_cert} does not chain to our CA {ours_fp} for {purpose}: "
                                       f"{last_line(err or out.decode(errors='replace'), 'verify failed')}")
                        break
                rc, _, _ = openssl("x509", "-in", cert, "-noout", "-checkend", a.min_valid_seconds)
                if rc:
                    refused.append(f"{name}: {src_cert} expires within {a.min_valid_seconds} seconds (or has "
                                   "expired). Re-run phase 8 with --rotate.")
            if len(refused) == failed:
                ok.append(f"{name}: CN={want}, key matches, chains to our CA {ours_fp}")

    return emit()


if __name__ == "__main__":
    sys.exit(main())
