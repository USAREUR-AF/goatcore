#!/usr/bin/env python3
"""assert-universal-read-coverage.py — refuse to narrow the universal read out
from under a live reader.

THE FAILURE THIS EXISTS TO STOP, as it happened on a production goatnet:

  `zenoh_universal_read_cns` lists the non-slot readers that keep fabric read
  when the B2.1 partner partition narrows `trial-peer-read` off link_protocols
  to an enumerated CN list. Miss a reader and it silently stops reading the
  whole fabric — no error, no log line, it just goes quiet.

  Its standup copied the first goatnet's list verbatim:
  panoscope-trial-peer, flashlight-trial-peer, names-collector-trial-peer,
  airdefenders-trial-peer. Reader cert CNs are minted PER GOATNET, and none of
  those four exist on the new goatnet — its readers are `panoscope`, `flashlight`,
  `stethoscope`. The list was 100% wrong and 100% silent, and it stayed that
  way for fifteen days because the partition never actually rendered (the lane
  map was empty). The first correct partner carve would have blinded three
  inspection surfaces at once.

  A list of names in a repo cannot be checked against a goatnet it was not
  measured on. So this runs on the TARGET HOST at deploy time, reads the certs
  that are really mounted there, and fails the play rather than let the render
  reach a router restart.

WHAT IT CHECKS. For every reader leaf cert on the host, the cert's CN must
appear somewhere in the rendered ACL's `cert_common_names`. If a CN appears
nowhere, that reader matches NO subject; once `trial-peer-read` is narrowed it
therefore has no read at all. That is the unambiguous failure and it is what we
fail on.

WHAT IT DELIBERATELY DOES NOT CHECK. Whether the subject a CN does match is
bound to a READ rule in the policies block. Deciding that needs a real JSON5
parse of `policies`, and the template already settles it (operator, infra-* and
the audit subscriber are bound to trial-peer-r-all by construction). Claiming
more precision than it has would make this guard a liar, which is worse than a
guard with a stated edge. The boundary is: covered-here means "has a subject",
not "provably reads".

INERT WHEN THE READ IS FLAT. If the rendered `trial-peer-read` subject still
carries `link_protocols` (partition off, or lane map empty), nothing can lose
read and the check reports and exits 0. It only bites when narrowing is real.

usage: assert-universal-read-coverage.py <rendered-config.json5> [cert-glob ...]
       --self-test    prove RED against that defect, GREEN when fixed
"""
import glob
import os
import re
import subprocess
import sys

# Where readers actually keep their leaf on a goatnet. This list is the guard's
# REACH, and a reader whose cert sits outside it is invisible here — the check
# then passes without having looked at it, which is a weaker answer than it
# appears. Known locations:
#
#   /etc/<svc>/zenoh-client-cert.pem   panoscope, flashlight, stethoscope,
#                                      fabric-health, the *-health publishers
#   /opt/<svc>/certs/<svc>-cert.pem    airdefenders (its zenoh sidecar
#                                      bind-mounts /opt/airdefenders/certs)
#
# Add a path here rather than assuming; the cost of missing one is a reader that
# goes silently dark on the next narrowing.
#   /opt/<svc>-certs/<svc>-cert.pem    goat-map and goat-map-synth, whose
#                                      external-service compose bind-mounts a
#                                      SIBLING directory rather than a certs/
#                                      subdirectory
DEFAULT_CERT_GLOBS = [
    "/etc/*/zenoh-client-cert.pem",
    "/etc/*/zenoh/client-cert.pem",
    "/etc/*/client-cert.pem",
    "/opt/*/certs/*-cert.pem",
    "/opt/*/certs/zenoh-client-cert.pem",
    "/opt/*-certs/*-cert.pem",
]


def _subject_issuer(path):
    """(subject, issuer) strings for a cert, or (None, None)."""
    try:
        out = subprocess.run(
            ["openssl", "x509", "-in", path, "-noout", "-subject", "-issuer"],
            capture_output=True, text=True, timeout=15,
        )
    except (OSError, subprocess.SubprocessError):
        return None, None
    if out.returncode != 0:
        return None, None
    subj = iss = None
    for line in out.stdout.splitlines():
        if line.startswith("subject"):
            subj = line.split("=", 1)[1].strip()
        elif line.startswith("issuer"):
            iss = line.split("=", 1)[1].strip()
    return subj, iss


def cert_cn(path):
    """CN of a READER LEAF, or None.

    Returns None for a CA/root certificate. The globs necessarily sweep whole
    cert directories, and a service's dir usually holds the CA root beside its
    leaf (e.g. /opt/fabric-health/certs/ca-root-cert.pem). A root is not a
    reader — nothing presents it as a client identity — so demanding it appear
    in the ACL would fail the play over a certificate that cannot read
    anything. Detected structurally (subject == issuer) rather than by
    filename, because the naming is not consistent across roles.
    """
    subj, iss = _subject_issuer(path)
    if subj is None:
        return None
    if iss is not None and subj == iss:
        return None  # self-signed: a CA root, not a reader leaf
    m = re.search(r"CN\s*=\s*([^,/\n]+)", subj)
    return m.group(1).strip() if m else None


def config_cns(text):
    """Every CN enumerated in any cert_common_names list in the rendered ACL."""
    found = set()
    for block in re.findall(r"cert_common_names\s*:\s*\[(.*?)\]", text, re.S):
        found.update(re.findall(r'"([^"]+)"', block))
    return found


def read_is_narrowed(text):
    """True when the trial-peer-read subject enumerates CNs instead of riding
    link_protocols. Matches the subject object that carries the id, so an
    unrelated later block cannot be mistaken for it."""
    m = re.search(
        r'\{\s*id:\s*"trial-peer-read"\s*,(.*?)\}', text, re.S
    )
    if not m:
        return False
    return "cert_common_names" in m.group(1)


def main(argv):
    cfg_path = argv[0]
    cert_globs = argv[1:] or DEFAULT_CERT_GLOBS

    with open(cfg_path, encoding="utf-8") as fh:
        text = fh.read()

    narrowed = read_is_narrowed(text)
    declared = config_cns(text)

    live = {}
    for pat in cert_globs:
        for p in sorted(glob.glob(pat)):
            cn = cert_cn(p)
            if cn:
                live.setdefault(cn, p)

    if not narrowed:
        print(f"OK (inert): {cfg_path} still grants trial-peer-read via link_protocols — "
              f"the read is flat, no reader can lose it. {len(live)} reader cert(s) seen.")
        return 0

    if not live:
        print(f"OK (nothing to check): read IS narrowed but no reader leaf cert was found "
              f"under {cert_globs}. If that is wrong, the globs are wrong, not the ACL.")
        return 0

    missing = {cn: p for cn, p in live.items() if cn not in declared}

    # Names carried in the ACL with no cert behind them on this host. Not fatal
    # on its own (an off-host reader is legitimate), but it is the fingerprint
    # of a list ported from another goatnet, so say so loudly.
    orphan_shaped = sorted(
        cn for cn in declared
        if cn not in live and not re.fullmatch(r"[0-9a-f]{32}", cn)
    )

    print(f"universal-read coverage: read IS narrowed in {cfg_path}")
    print(f"  live reader certs on this host : {len(live)}")
    print(f"  CNs enumerated in the ACL      : {len(declared)}")
    for cn, p in sorted(live.items()):
        print(f"    {'OK  ' if cn not in missing else 'MISS'}  {cn}   ({p})")

    if orphan_shaped:
        print("  NOTE: named in the ACL but no cert on this host — fine for a genuinely")
        print("        off-host reader, and the exact fingerprint of a list copied from")
        print("        another goatnet (reader CNs are minted per goatnet):")
        for cn in orphan_shaped:
            print(f"          {cn}")

    if missing:
        print()
        print("FAIL: narrowing this read would silently blind the reader(s) below — they")
        print("      match no subject in the rendered ACL, so once trial-peer-read stops")
        print("      granting on link_protocols they read NOTHING, with no error anywhere.")
        print("      The router has NOT been restarted; the ACL in force is still")
        print("      the previous one.")
        for cn, p in sorted(missing.items()):
            print(f"        {cn}   ({p})")
        print()
        print("      Fix: add the CN(s) above to zenoh_universal_read_cns in THIS goatnet's")
        print("      group_vars — measured here, not copied from another goatnet — then")
        print("      re-run. If a reader is genuinely being retired, remove its cert first.")
        return 1

    print("OK: every live reader CN is covered by a subject in the rendered ACL")
    return 0


def self_test():
    import tempfile
    tmp = tempfile.mkdtemp()
    narrowed = '''
    subjects: [
      {
        id: "trial-peer-read",
        cert_common_names: [
          "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          "panoscope-trial-peer",
          "airdefenders-trial-peer",
        ],
      },
      {
        id: "infra-fabric-health",
        cert_common_names: ["fabric-health-trial-peer"],
      },
    ],
    '''
    flat = '''
    subjects: [
      {
        id: "trial-peer-read",
        link_protocols: ["tls"],
      },
    ],
    '''
    ok = True

    # Fake the cert scan by monkeypatching cert_cn: the CNs a goatnet really has.
    global cert_cn
    real = cert_cn
    cert_cn = lambda p: os.path.basename(os.path.dirname(p))  # noqa: E731

    def write(name, body):
        p = os.path.join(tmp, name)
        with open(p, "w", encoding="utf-8") as fh:
            fh.write(body)
        return p

    for cn in ("panoscope", "flashlight", "stethoscope"):
        d = os.path.join(tmp, "certs", cn)
        os.makedirs(d, exist_ok=True)
        write(os.path.join(d, "zenoh-client-cert.pem"), "x")
    cglob = os.path.join(tmp, "certs", "*", "zenoh-client-cert.pem")

    # RED — a narrowed ACL naming another goatnet's CNs.
    rc = main([write("narrowed.json5", narrowed), cglob])
    if rc == 0:
        print("SELF-TEST FAIL: passed the known defect (narrowed ACL, wrong CNs)")
        ok = False
    else:
        print("self-test: RED proven — narrowed ACL missing live reader CNs fails")

    # GREEN — same ACL with the live CNs added.
    fixed = narrowed.replace(
        '"airdefenders-trial-peer",',
        '"panoscope",\n          "flashlight",\n          "stethoscope",',
    )
    if main([write("fixed.json5", fixed), cglob]) != 0:
        print("SELF-TEST FAIL: rejected an ACL that covers every live reader")
        ok = False
    else:
        print("self-test: GREEN proven — narrowed ACL covering every live CN passes")

    # GREEN — flat read is inert even with the wrong list.
    if main([write("flat.json5", flat), cglob]) != 0:
        print("SELF-TEST FAIL: flat (un-narrowed) read must be inert")
        ok = False
    else:
        print("self-test: GREEN proven — flat read is inert")

    cert_cn = real
    print("OK: universal-read-coverage self-test — red-first proof holds" if ok else "SELF-TEST FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--self-test":
        sys.exit(self_test())
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(2)
    sys.exit(main(sys.argv[1:]))
