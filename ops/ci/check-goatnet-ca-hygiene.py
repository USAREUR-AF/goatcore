#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# check-goatnet-ca-hygiene.py — does each goatnet have its own trust root, and
# is the shape of its leaves written down anywhere?
#
# WHY THIS EXISTS. A root cert's path says nothing about which key it holds, and
# two roots in this repo can be DIFFERENT KEYS WITH THE SAME COMMON NAME. A cert
# minted against the wrong one verifies perfectly against the root you thought
# you were using and fails at the router with `received fatal alert:
# DecryptError`, which names neither the CA nor the mismatch.
#
# Two conditions make that possible, and this script reports both:
#
#   1. A goatnet whose router trusts a root belonging to a DIFFERENT goatnet.
#      The `goat-zenoh` role's default root path, if unset by group_vars, points
#      at another goatnet's root. Sharing a root means a leaf
#      minted for one goatnet is trusted by another's router.
#
#   2. A goatnet with no cert manifest under ops/enrollment/manifests/. Without
#      one, the required shape of a leaf can only be reverse-engineered from a
#      live cert — and a re-issue that quietly adds or drops a SAN is a change,
#      not a rotation.
#
# WARN, NOT FAIL, and deliberately. Every finding below predates this check;
# failing today would block unrelated PRs on debt their authors did not create,
# and a gate people route around is worse than a report people read. It reports
# to stdout and exits 0. Promote to a hard failure once the listed goatnets are
# clean — the promotion is one constant, PROMOTED_TO_FAILURE, and the list it
# guards is derived, not hand-maintained, so a NEW goatnet cannot be
# grandfathered by accident.
#
# Usage:
#   ops/ci/check-goatnet-ca-hygiene.py           # report, exit 0
#   ops/ci/check-goatnet-ca-hygiene.py --strict  # exit 1 on any finding

import argparse
import os
import re
import subprocess
import sys
from collections import defaultdict

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
GROUP_VARS = os.path.join(REPO, "ops", "ansible", "group_vars")
CA_DIR = os.path.join(REPO, "ops", "enrollment", "ca")
MANIFESTS = os.path.join(REPO, "ops", "enrollment", "manifests")
ROLE_DEFAULTS = os.path.join(
    REPO, "ops", "ansible", "roles", "goat-zenoh", "defaults", "main.yml")

PROMOTED_TO_FAILURE = False


def sh(*args):
    return subprocess.run(args, capture_output=True, text=True).stdout.strip()


def role_default_root():
    """The root a goatnet gets when its group_vars says nothing."""
    try:
        with open(ROLE_DEFAULTS) as fh:
            m = re.search(r"^goat_ca_root_cert_path:\s*\"?([^\"\n]+)", fh.read(), re.M)
    except OSError:
        return None
    if not m:
        return None
    rel = re.sub(r".*?enrollment/", "ops/enrollment/", m.group(1)).strip()
    return rel


def router_goatnets():
    """Groups that actually run a zenoh router — they define `zenoh_stack`.

    Not every group_vars directory is a goatnet; obs/standby/backup groups are
    sub-groups sharing their parent's fabric. Only a router terminates mTLS, so
    only a router's group needs its own root.
    """
    out = {}
    if not os.path.isdir(GROUP_VARS):
        return out
    for d in sorted(os.listdir(GROUP_VARS)):
        p = os.path.join(GROUP_VARS, d)
        if not os.path.isdir(p):
            continue
        for f in sorted(os.listdir(p)):
            if not f.endswith((".yml", ".yaml")):
                continue
            try:
                with open(os.path.join(p, f)) as fh:
                    text = fh.read()
            except OSError:
                continue
            if re.search(r"^zenoh_stack:", text, re.M):
                out.setdefault(d, None)
            m = re.search(r"^goat_ca_root_cert_path:\s*\"?([^\"\n]+)", text, re.M)
            if m and d in out:
                out[d] = re.sub(r".*?enrollment/", "ops/enrollment/", m.group(1)).strip()
    return out


def cn_of(path):
    subj = sh("openssl", "x509", "-in", os.path.join(REPO, path), "-noout", "-subject")
    m = re.search(r"CN\s*=\s*(.+)$", subj)
    return m.group(1).strip() if m else "?"


def fingerprint(path):
    out = sh("openssl", "x509", "-in", os.path.join(REPO, path),
             "-noout", "-fingerprint", "-sha256")
    return out.split("=", 1)[1] if "=" in out else "?"


def goatnet_of(group):
    """The goatnet a router group belongs to, and the root that goatnet owns.

    Groups are not goatnets. A goatnet's second host, its cloud host site and a
    site inside it are separate groups that legitimately share the goatnet's
    root, and a check that called that "not its own root" would be reporting the
    design as a defect.

    A goatnet is identified by OWNING a root: the longest prefix of the group
    name for which ops/enrollment/ca/<name>/root-cert.pem exists. That is
    derived, so standing up a goatnet with its own CA moves it out of the
    findings automatically, and a longer-named goatnet resolves to itself rather
    than folding into a shorter one because its own directory is the longer match.
    """
    env = re.sub(r"_hosts$", "", group)
    parts = env.split("_")
    for i in range(len(parts), 0, -1):
        cand = "-".join(parts[:i])
        rel = f"ops/enrollment/ca/{cand}/root-cert.pem"
        if os.path.exists(os.path.join(REPO, rel)):
            return cand, rel
    return env.replace("_", "-"), None


def manifest_for(group):
    env = re.sub(r"_hosts$", "", group).replace("_", "-")
    rel = f"ops/enrollment/manifests/{env}-certs.yaml"
    return rel, os.path.exists(os.path.join(REPO, rel))


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--strict", action="store_true",
                    help="exit 1 on any finding (what promotion will make the default)")
    args = ap.parse_args()

    findings = []
    default_root = role_default_root()

    # --- 1. roots that cannot be told apart by name -------------------------
    by_cn = defaultdict(list)
    if os.path.isdir(CA_DIR):
        for d in sorted(os.listdir(CA_DIR)):
            rel = f"ops/enrollment/ca/{d}/root-cert.pem"
            if os.path.exists(os.path.join(REPO, rel)):
                by_cn[cn_of(rel)].append(rel)

    print("== trust roots in this repo ==")
    for cn, paths in sorted(by_cn.items()):
        if len(paths) > 1:
            findings.append(f"{len(paths)} roots share CN={cn!r}")
            print(f"  {cn!r}: {len(paths)} roots — NAME DOES NOT IDENTIFY THEM")
            for p in paths:
                print(f"      {p}")
                print(f"        sha256 {fingerprint(p)}")
        else:
            print(f"  {cn!r}: {paths[0]}")
    print()

    # --- 2. per-goatnet root and manifest ------------------------------------
    nets = router_goatnets()
    print("== goatnets that run a zenoh router ==")
    if default_root:
        print(f"   (role default when a group says nothing: {default_root})")
    print()
    for group in sorted(nets):
        root = nets[group] or default_root
        net, own_root = goatnet_of(group)
        owns = bool(root) and bool(own_root) and root == own_root
        rel_manifest, has_manifest = manifest_for(group)

        print(f"  {group}   (goatnet: {net})")
        if owns:
            print(f"    root:     {root}")
        elif own_root:
            print(f"    root:     {root or 'UNRESOLVED'}")
            print(f"              <-- goatnet {net} OWNS {own_root} and does not use it")
            findings.append(
                f"goatnet {net} owns {own_root} but {group} trusts {root}")
        else:
            print(f"    root:     {root or 'UNRESOLVED'}")
            print(f"              <-- goatnet {net} has NO root of its own")
            findings.append(f"goatnet {net} has no root of its own ({group} trusts {root})")
        print(f"    manifest: {rel_manifest if has_manifest else 'ABSENT — ' + rel_manifest}")
        if not has_manifest:
            findings.append(f"{group} has no cert manifest")
        print()

    # --- verdict -------------------------------------------------------------
    if not findings:
        print("ok: every router-running goatnet has its own trust root and a cert "
              "manifest, and no two roots share a Common Name.")
        return 0

    print("-" * 72)
    print(f"{len(findings)} finding(s):")
    for f in findings:
        print(f"  - {f}")
    print()

    # A warn-level job that only writes to a log is a job nobody reads. GitHub
    # renders ::warning:: as an annotation on the run and in the PR checks
    # panel, which is the whole point of running this before it can fail.
    if os.environ.get("GITHUB_ACTIONS"):
        for f in findings:
            print(f"::warning title=goatnet CA hygiene::{f}")
    print("WHAT THESE COST, measured 2026-09-05: re-issuing one ZAGROS leaf took a")
    print("four-minute outage because the root the router trusts is the nova")
    print("one, not the identically-named `dev` one, and a cert minted against the")
    print("wrong root verifies fine locally then fails at the router with")
    print("`received fatal alert: DecryptError` — naming neither the CA nor the")
    print("mismatch. A missing manifest is why the leaf's shape had to be")
    print("reverse-engineered from the live cert in the first place.")
    print()
    print("FIX: give the goatnet its own root (ca-init), re-issue its leaves")
    print("against it while both roots are trusted, then drop the shared one; and")
    print("write ops/enrollment/manifests/<env>-certs.yaml before the first mint.")
    print("Plan: docs/design/credential-hygiene-strategy.md")

    if args.strict or PROMOTED_TO_FAILURE:
        return 1
    print()
    print("WARNING ONLY — every finding above predates this check. It exits 0 so")
    print("it does not block PRs on debt their authors did not create. Flip")
    print("PROMOTED_TO_FAILURE once the list is clean; the list is derived, so a")
    print("NEW goatnet cannot be grandfathered by accident.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
