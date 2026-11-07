#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""check-peering-forwarder-holds-nothing.py — the peering forwarder cannot be handed
peering material.

A peering forwarder relays ciphertext and holds nothing of the peering: no peering
WireGuard key, no CA, no SVID, no fabric certificate, no trust bundle, no mesh
enrolment (docs/design/peering-forwarder.md §2, ADR 1218). Its threat model rests on
that: a compromised forwarder can drop and count, and cannot read or inject, only
because it has no key. This guard keeps the role from ever being able to receive one.

It checks, statically, roles/goat-peering-forwarder and ops/ansible/peering-forwarder.yml:

  1. Every variable the role declares (defaults/, vars/) is named peering_forwarder_*
     and no name carries a CA, certificate, SVID, SPIFFE, trust-bundle, TLS, fabric,
     NetBird or setup-key token.
  2. No Jinja expression or conditional in tasks/, handlers/, templates/ or meta/
     references another role's peering, fabric, federation, SPIRE or NetBird
     variables.
  3. No task reads or ships a file from outside the role: no lookup(), no
     include_vars, no vars_files, no include_role/import_role, no copy from a src
     file, no fetch, and nothing names a .pem/.crt/.der/.p12/.pfx/.csr file or a
     NetBird, SPIRE or edge configuration path.
  4. meta/main.yml declares no dependencies.
  5. The playbook runs exactly this role, with no vars_files, pre_tasks or extra roles.

Usage:
  ops/ci/check-peering-forwarder-holds-nothing.py                # the real role
  ops/ci/check-peering-forwarder-holds-nothing.py --role DIR --playbook FILE
  ops/ci/check-peering-forwarder-holds-nothing.py --self-test    # every fixture is refused
"""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys

try:
    import yaml
except ImportError:  # CI installs PyYAML; refuse rather than pass silently
    print("check-peering-forwarder-holds-nothing: PyYAML is required", file=sys.stderr)
    sys.exit(2)

ROLE = "ops/ansible/roles/goat-peering-forwarder"
PLAYBOOK = "ops/ansible/peering-forwarder.yml"
FIXTURES = "ops/ci/fixtures/peering-forwarder-holds-nothing"

# Tokens a forwarder variable name must never carry (case-insensitive, on _ words).
NAME_TOKENS = {
    "ca", "cacert", "cert", "certs", "certificate", "pem", "crt", "svid", "spiffe",
    "spire", "bundle", "trust", "tls", "mtls", "fabric", "zenoh", "netbird",
    "setup", "setupkey", "psk", "preshared", "privkey", "private", "leaf", "pki",
}
# Identifiers no forwarder expression may reference: another role's material.
FOREIGN_IDENT = re.compile(
    r"\b(peering_wg_\w+|goat_peering_\w+|goat_zenoh\w*|zenoh_\w+|fed_\w+|federation_\w+|"
    r"netbird\w*|nb_\w+|spire\w*|spiffe\w*|svid\w*|\w*_ca\b|\w*_ca_\w+|\w*cert\w*|"
    r"\w*trust_bundle\w*|\w*setup_key\w*)\b"
)
# Text no task, handler, template or meta file may contain, comments excluded.
FORBIDDEN_TEXT = [
    (re.compile(r"\blookup\s*\("), "lookup() reads a file from the control node"),
    (re.compile(r"\bquery\s*\("), "query() reads from the control node"),
    (re.compile(r"\binclude_vars\b"), "include_vars brings in another file's variables"),
    (re.compile(r"\bvars_files\b"), "vars_files brings in another file's variables"),
    (re.compile(r"\b(include|import)_role\b"), "another role can carry material in"),
    (re.compile(r"\bansible\.builtin\.fetch\b|^\s*fetch:", re.M), "fetch moves files off the host"),
    (re.compile(r"\bremote_src\b"), "remote_src copies a file already on the host"),
    (re.compile(r"\b[\w./-]+\.(pem|crt|cer|der|p12|pfx|csr)\b"), "names a certificate or key file"),
    (re.compile(r"/etc/netbird|/var/lib/netbird|\bnetbird\s+up\b"), "a NetBird path or enrolment"),
    (re.compile(r"/etc/spire|/run/spire|/opt/spire"), "a SPIRE path"),
    (re.compile(r"/etc/goat-peering-wg|/etc/goat-zenoh|/etc/goat-peering-edge"), "an edge's configuration path"),
]
EXPR_KEYS = {"when", "that", "failed_when", "changed_when", "until", "loop", "with_items"}


def strip_comments(text: str) -> str:
    text = re.sub(r"\{#.*?#\}", "", text, flags=re.S)
    out = []
    for line in text.splitlines():
        s = line.lstrip()
        if s.startswith("#") and not s.startswith("#!"):
            continue
        out.append(line)
    return "\n".join(out)


def jinja_expressions(text: str) -> list[str]:
    return re.findall(r"\{\{(.*?)\}\}", text, flags=re.S) + re.findall(r"\{%(.*?)%\}", text, flags=re.S)


def yaml_expressions(node) -> list[str]:
    out: list[str] = []
    if isinstance(node, dict):
        for k, v in node.items():
            if k in EXPR_KEYS:
                out.extend(str(x) for x in (v if isinstance(v, list) else [v]))
            out.extend(yaml_expressions(v))
    elif isinstance(node, list):
        for v in node:
            out.extend(yaml_expressions(v))
    return out


def walk(root: str, sub: str) -> list[str]:
    base = os.path.join(root, sub)
    files = []
    for d, _, names in os.walk(base):
        files.extend(os.path.join(d, n) for n in sorted(names))
    return sorted(files)


def check(role: str, playbook: str) -> list[str]:
    errs: list[str] = []
    if not os.path.isdir(role):
        return [f"{role}: no such role"]

    # 1. declared names
    for f in walk(role, "defaults") + walk(role, "vars"):
        try:
            with open(f, encoding="utf-8") as stream:
                doc = yaml.safe_load(stream) or {}
        except yaml.YAMLError as e:
            errs.append(f"{f}: not YAML: {e}")
            continue
        if not isinstance(doc, dict):
            continue
        names = list(doc)
        nested = doc.get("peering_forwarder_peering_defaults")
        if isinstance(nested, dict):
            names += [f"peering_forwarder_peering_defaults.{k}" for k in nested]
        for n in names:
            top = n.split(".")[0]
            if not top.startswith("peering_forwarder_"):
                errs.append(f"{f}: variable {n!r} is not named peering_forwarder_*")
            words = set(re.split(r"[_.]+", n.lower()))
            hit = sorted(words & NAME_TOKENS)
            if hit:
                errs.append(f"{f}: variable {n!r} carries {hit}: a forwarder holds no such material")
        for v in doc.values():
            for e in jinja_expressions(str(v)):
                for m in FOREIGN_IDENT.findall(e):
                    errs.append(f"{f}: a default references {m!r}, another role's material")

    # 2 + 3. expressions and text
    for sub in ("tasks", "handlers", "templates", "meta"):
        for f in walk(role, sub):
            base = os.path.basename(f).lower()
            for tok in ("ca", "cert", "svid", "spire", "spiffe", "bundle", "tls", "netbird", "pem"):
                if re.search(rf"(^|[^a-z]){tok}([^a-z]|$)", base):
                    errs.append(f"{f}: file name carries {tok!r}")
            with open(f, encoding="utf-8") as stream:
                raw = stream.read()
            text = strip_comments(raw)
            exprs = jinja_expressions(text)
            if f.endswith((".yml", ".yaml")):
                try:
                    exprs += yaml_expressions(yaml.safe_load(raw))
                except yaml.YAMLError as e:
                    errs.append(f"{f}: not YAML: {e}")
                    continue
                if re.search(r"^\s*(ansible\.builtin\.)?copy:\s*$", text, re.M) and re.search(r"^\s*src:", text, re.M):
                    for blk in re.split(r"\n\s*- name:", text):
                        if re.search(r"(ansible\.builtin\.)?copy:", blk) and re.search(r"^\s*src:", blk, re.M):
                            errs.append(f"{f}: a copy task ships a src file; a forwarder takes none")
                            break
            for e in exprs:
                for m in sorted(set(FOREIGN_IDENT.findall(e))):
                    errs.append(f"{f}: references {m!r}, another role's material")
            for rx, why in FORBIDDEN_TEXT:
                m = rx.search(text)
                if m:
                    errs.append(f"{f}: {m.group(0).strip()!r}: {why}")

    # 4. no dependencies
    meta = os.path.join(role, "meta", "main.yml")
    if os.path.isfile(meta):
        with open(meta, encoding="utf-8") as stream:
            doc = yaml.safe_load(stream) or {}
        if doc.get("dependencies"):
            errs.append(f"{meta}: declares dependencies {doc['dependencies']!r}; a forwarder role has none")
    if os.path.isdir(os.path.join(role, "files")) and walk(role, "files"):
        errs.append(f"{role}/files: a forwarder role ships no files")

    # 5. the playbook
    if playbook:
        try:
            with open(playbook, encoding="utf-8") as stream:
                plays = yaml.safe_load(stream) or []
        except (OSError, yaml.YAMLError) as e:
            return errs + [f"{playbook}: unreadable: {e}"]
        want = os.path.basename(os.path.normpath(role))
        for p in plays:
            for k in ("vars_files", "pre_tasks", "post_tasks", "tasks", "vars", "include_vars"):
                if k in p:
                    errs.append(f"{playbook}: play has {k!r}; it runs the role and nothing else")
            roles = [r["role"] if isinstance(r, dict) else r for r in p.get("roles", [])]
            if roles != [want]:
                errs.append(f"{playbook}: roles {roles!r}; it runs exactly [{want!r}]")
            for r in p.get("roles", []):
                if isinstance(r, dict) and set(r) - {"role"}:
                    errs.append(f"{playbook}: role entry passes {sorted(set(r) - {'role'})}; it passes nothing")
    return errs


def self_test(root: str) -> int:
    fx = os.path.join(root, FIXTURES)
    cases = sorted(d for d in os.listdir(fx) if os.path.isdir(os.path.join(fx, d)))
    if not cases:
        print("self-test: no fixtures", file=sys.stderr)
        return 1
    rc = 0
    for c in cases:
        d = os.path.join(fx, c)
        pb = os.path.join(d, "playbook.yml")
        errs = check(os.path.join(d, "goat-peering-forwarder"), pb if os.path.isfile(pb) else "")
        if errs:
            print(f"  refused  {c}: {errs[0]}")
        else:
            print(f"  ACCEPTED {c}: the guard let this fixture through", file=sys.stderr)
            rc = 1
    real = check(os.path.join(root, ROLE), os.path.join(root, PLAYBOOK))
    if real:
        print("  the real role fails:\n    " + "\n    ".join(real), file=sys.stderr)
        rc = 1
    print(f"check-peering-forwarder-holds-nothing --self-test: {len(cases)} fixture(s) {'ok' if rc == 0 else 'FAILED'}")
    return rc


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--role")
    ap.add_argument("--playbook")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    try:
        root = subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip()
    except Exception:
        root = os.getcwd()
    if a.self_test:
        return self_test(root)
    errs = check(a.role or os.path.join(root, ROLE), a.playbook if a.playbook is not None else os.path.join(root, PLAYBOOK))
    if errs:
        print("check-peering-forwarder-holds-nothing: FAIL")
        print("  " + "\n  ".join(errs))
        print("  A forwarder holds nothing of the peering: docs/design/peering-forwarder.md §2")
        return 1
    print("check-peering-forwarder-holds-nothing: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
