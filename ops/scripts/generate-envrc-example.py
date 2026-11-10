#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# generate-envrc-example.py — derive .envrc.example from what the repo actually
# reads, instead of from what somebody remembered to write down.
#
# WHY THIS IS GENERATED. The hand-written file documented 7 of 537 live
# variables and 6 of those 7 named variables nothing consumes any
# more. A checklist that is 1% complete and 86% wrong is worse than no checklist,
# because someone standing up a goatnet will believe it. The consumers are the
# only honest source: a variable matters exactly when something reads it.
#
# WHAT IT READS
#   ansible   {{ lookup('env', 'NAME') }}          both quote styles
#   python    os.environ["NAME"] / .get("NAME") / os.getenv("NAME")
#   go        os.Getenv("NAME")
#   shell     ${NAME:?...} and ${NAME:-...}        the *required*/defaulted forms
#             only — a bare $NAME is unbounded noise and is deliberately missed
#
# WHAT IT DOES NOT READ, and therefore cannot promise: a variable consumed only
# through indirection (a name built at runtime, a var-of-vars) is invisible here.
# `--check` compares against the committed file so drift is caught; it cannot
# catch a variable no code names literally.
#
# THE PER-GOATNET SECTION is the part that matters for standing up a new
# environment. Most secrets come in families — ZAGROS_PROD_PG_PASS,
# VEGA_PG_PASS, NOVA_PG_PASS — so the file emits the FAMILY once as
# `<ENV>_PG_PASS` rather than once per goatnet, and lists the prefixes seen.
# That turns 537 lines nobody reads into a per-goatnet checklist somebody can.
#
# NO VALUES. This file names variables and points at their consumers. It never
# reads .envrc.local.d, which is gitignored and absent in CI by design.
#
# Usage:
#   ops/scripts/generate-envrc-example.py            # write .envrc.example
#   ops/scripts/generate-envrc-example.py --check    # CI: fail if it would change
#   ops/scripts/generate-envrc-example.py --stdout   # print, write nothing

import argparse
import os
import re
import subprocess
import sys
from collections import defaultdict

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(REPO, ".envrc.example")

# Names that are the shell's, the OS's, or CI's — never ours to document.
NOT_OURS = {
    "PATH", "HOME", "USER", "SHELL", "TERM", "LANG", "LC_ALL", "PWD", "OLDPWD",
    "TMPDIR", "EDITOR", "VISUAL", "PAGER", "HOSTNAME", "LOGNAME", "SHLVL",
    "CI", "RUNNER_OS", "RUNNER_TEMP", "GITHUB_TOKEN", "GITHUB_ACTIONS",
    "GITHUB_WORKSPACE", "GITHUB_REPOSITORY", "GITHUB_SHA", "GITHUB_REF",
    "GITHUB_OUTPUT", "GITHUB_ENV", "GITHUB_EVENT_NAME", "GITHUB_RUN_ID",
    "GITHUB_STEP_SUMMARY",
    "PYTHONPATH", "PYTHONUNBUFFERED", "GOPATH", "GOCACHE", "GOFLAGS",
    "DOCKER_HOST", "DOCKER_BUILDKIT", "COLUMNS", "LINES", "TZ", "SSH_AUTH_SOCK",
    "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_RUNTIME_DIR", "DISPLAY",
    "AWS_REGION", "AWS_DEFAULT_REGION", "AWS_PROFILE", "AWS_VAULT",
    "OBJC_DISABLE_INITIALIZE_FORK_SAFETY", "ANSIBLE_CONFIG", "ANSIBLE_FORCE_COLOR",
    "NO_COLOR", "FORCE_COLOR", "DEBUG", "VERBOSE", "TRACE",
    # Placeholders that appear in prose and examples, never as real names.
    "NAME", "VARNAME", "VALUE", "EXAMPLE", "YOUR_VAR", "SOME_VAR",
}

# Directories with no bearing on standing an environment up.
SKIP_DIRS = {".git", "node_modules", "vendor", "target", "dist", "build",
             ".claude", "testdata", "fixtures", "fixture-e2e", "tests", "archived"}

PATTERNS = [
    # (compiled regex, capture group index, source label)
    (re.compile(r"lookup\(\s*['\"]env['\"]\s*,\s*['\"]([A-Z][A-Z0-9_]{3,})['\"]"), 1, "ansible"),
    (re.compile(r"os\.environ(?:\.get)?\(?\[?\s*['\"]([A-Z][A-Z0-9_]{3,})['\"]"), 1, "python"),
    (re.compile(r"os\.getenv\(\s*['\"]([A-Z][A-Z0-9_]{3,})['\"]"), 1, "python"),
    (re.compile(r"os\.Getenv\(\s*\"([A-Z][A-Z0-9_]{3,})\""), 1, "go"),
    # Shell: only the forms that DECLARE a requirement or a default. A bare
    # $NAME would sweep in every loop variable in the repo.
    (re.compile(r"\$\{([A-Z][A-Z0-9_]{3,})(?::[?-])"), 1, "shell"),
]

SECRETISH = re.compile(
    r"(PASS|PASSWORD|SECRET|TOKEN|KEY|ENCKEY|PASSPHRASE|_PW$|CIPHER|COOKIE|"
    r"CSRF|SALT|CREDENTIAL|PRIVATE)"
)

def env_prefixes_from_repo():
    """The environment prefixes, taken from the repo's own list of goatnets.

    Inferring them from the variable names themselves does not work: HYDRA,
    KRATOS, RELAY and PORTAL all front several suffixes and look exactly like
    environment prefixes to a frequency heuristic, which is how a first cut of
    this script decided `<ENV>_ADMIN` was a per-goatnet variable "in use by
    HYDRA, KRATOS". They are COMPONENT prefixes.

    `ops/ansible/group_vars/<name>_hosts/` is the repo's actual register of
    goatnets and site groups, so a new goatnet updates this list by existing
    rather than by someone editing this file.
    """
    gv = os.path.join(REPO, "ops", "ansible", "group_vars")
    out = set()
    try:
        entries = os.listdir(gv)
    except OSError:
        return out
    for d in entries:
        if not os.path.isdir(os.path.join(gv, d)):
            continue
        base = re.sub(r"_(hosts|obs_hosts|standby_hosts)$", "", d)
        if base and base != d:
            out.add(base.upper())
    return out


# This file documents the patterns it searches for, so it matches itself: the
# `${NAME:?...}` in the header above is a five-character capture, and the first
# committed run duly reported a variable called NAME "in use by
# generate-envrc-example.py". Its own CI gate caught it. A scanner must not read
# its own examples as evidence.
SELF = os.path.relpath(os.path.abspath(__file__), REPO)


def tracked_files():
    # An assembled tree (.goat-assembled at its root) holds its layers
    # uncommitted over goatcore's HEAD, so their consumers are untracked there:
    # it is read as tracked plus untracked, ignored files left out, or a
    # deployment's own variables would drop out of the file it documents.
    cmd = ["git", "-C", REPO, "ls-files"]
    if os.path.isfile(os.path.join(REPO, ".goat-assembled")):
        cmd += ["-co", "--exclude-standard"]
    out = subprocess.run(cmd, capture_output=True, text=True, check=True).stdout
    for rel in out.splitlines():
        if rel == SELF:
            continue
        parts = rel.split("/")
        if any(p in SKIP_DIRS for p in parts):
            continue
        if rel.endswith((".png", ".jpg", ".pdf", ".docx", ".pptx", ".zip",
                         ".gz", ".ico", ".woff", ".woff2", ".svg")):
            continue
        yield rel


def scan():
    """{VAR: {"sources": {label}, "files": {relpath}}}"""
    found = defaultdict(lambda: {"sources": set(), "files": set()})
    for rel in tracked_files():
        path = os.path.join(REPO, rel)
        try:
            with open(path, "r", encoding="utf-8", errors="replace") as fh:
                text = fh.read()
        except (OSError, IsADirectoryError):
            continue
        for rx, grp, label in PATTERNS:
            for m in rx.finditer(text):
                name = m.group(grp)
                if name in NOT_OURS:
                    continue
                found[name]["sources"].add(label)
                found[name]["files"].add(rel)
    return found


def split_families(names):
    """Return (families, singletons).

    A family is a set of variables that differ only by a leading environment
    prefix: ZAGROS_PROD_PG_PASS and NOVA_PG_PASS are one family, `<ENV>_PG_PASS`.
    The prefixes come from the repo's goatnet register, not from the variable
    names — see env_prefixes_from_repo() for why inference does not work here.
    """
    env_prefixes = env_prefixes_from_repo()
    families = defaultdict(set)
    for n in names:
        # Longest matching goatnet prefix wins: ZAGROS_PROD before ZAGROS, so
        # ZAGROS_PROD_PG_PASS is not filed under ZAGROS as "PROD_PG_PASS".
        for p in sorted(env_prefixes, key=len, reverse=True):
            if n.startswith(p + "_"):
                families[n[len(p) + 1:]].add(p)
                break
    families = {s: sorted(p) for s, p in families.items()}

    claimed = {f"{p}_{s}" for s, pfxs in families.items() for p in pfxs}
    singles = sorted(n for n in names if n not in claimed)
    return families, singles, sorted(env_prefixes)


def render(found):
    names = sorted(found)
    families, singles, env_prefixes = split_families(names)
    secret_count = sum(1 for n in names if SECRETISH.search(n))

    L = []
    a = L.append
    a("# .envrc.example — GENERATED. Do not edit by hand.")
    a("#")
    a("#   regenerate:  ops/scripts/generate-envrc-example.py")
    a("#   CI gate:     ops/scripts/generate-envrc-example.py --check")
    a("#")
    a("# Derived from what the repository actually reads — every ansible")
    a("# lookup('env'), python os.environ/getenv, go os.Getenv, and the shell")
    a("# ${VAR:?} / ${VAR:-} forms. A variable appears here because something")
    a("# consumes it, which is the only claim this file is entitled to make.")
    a("#")
    a(f"# {len(names)} variables, {secret_count} of them secret-bearing by name.")
    a("#")
    a("# THIS FILE HOLDS NO VALUES AND NEVER WILL. Credentials live in the store.")
    a("# .envrc.local.d/*.refs hold references, mode 0600 (ADR 1186).")
    a("# Legacy *.envrc values load only during the references-only migration.")
    a("# Layout and the file split: docs/operations/local-secrets-pattern.md.")
    a("# Blast radius, rotation cost and what cannot be rotated:")
    a("# docs/design/credential-inventory.md.")
    a("#")
    a("# A variable consumed only through indirection — a name assembled at")
    a("# runtime — is invisible to the scan and therefore absent here. This file")
    a("# is a floor, not a census.")
    a("")

    if families:
        a("# " + "=" * 74)
        a("# PER-GOATNET — the checklist for standing up a new environment.")
        a("#")
        a("# These come in families: one variable per goatnet, same suffix. Set")
        a("# every line below with your new environment's prefix substituted for")
        a("# <ENV>. Prefixes currently in use:")
        a("#")
        for chunk in [env_prefixes[i:i + 6] for i in range(0, len(env_prefixes), 6)]:
            a("#   " + "  ".join(chunk))
        a("# " + "=" * 74)
        a("")
        for sfx in sorted(families):
            pfxs = families[sfx]
            mark = "  # SECRET" if SECRETISH.search(sfx) else ""
            a(f"# in use by: {', '.join(pfxs)}")
            a(f"export <ENV>_{sfx}=\"\"{mark}")
            a("")

    # Secrets first. An undifferentiated list of 800+ names buries the twenty
    # that decide whether a goatnet is compromised, and the reader who most
    # needs this file is looking for exactly those.
    sec = [n for n in singles if SECRETISH.search(n)]
    cfg = [n for n in singles if not SECRETISH.search(n)]

    for title, group, note in (
        ("SINGLETON SECRETS — one value for the operator laptop, not per goatnet.",
         sec,
         "Every line here is credential material. Blast radius and rotation cost "
         "per class: docs/design/credential-inventory.md."),
        ("SINGLETON CONFIGURATION — addresses, paths, toggles. Not secrets.",
         cfg,
         "Named here because something reads them, so a missing one breaks a run "
         "the same way a missing secret does."),
    ):
        a("# " + "=" * 74)
        a(f"# {title}")
        a("#")
        for line in [note[i:i + 72] for i in range(0, len(note), 72)]:
            a("# " + line)
        a(f"# {len(group)} variables.")
        a("# " + "=" * 74)
        a("")
        for n in group:
            meta = found[n]
            where = sorted(meta["files"])[:2]
            mark = "  # SECRET" if SECRETISH.search(n) else ""
            a(f"# {', '.join(where)}" + (" …" if len(meta["files"]) > 2 else ""))
            a(f"export {n}=\"\"{mark}")
            a("")

    return "\n".join(L).rstrip() + "\n"


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true",
                    help="exit 1 if the committed file is not what this would write")
    ap.add_argument("--stdout", action="store_true", help="print, write nothing")
    args = ap.parse_args()

    body = render(scan())

    if args.stdout:
        sys.stdout.write(body)
        return 0

    if args.check:
        try:
            with open(OUT, encoding="utf-8") as fh:
                current = fh.read()
        except FileNotFoundError:
            print("FAIL: .envrc.example is missing. Run "
                  "ops/scripts/generate-envrc-example.py", file=sys.stderr)
            return 1
        if current != body:
            print("FAIL: .envrc.example is stale — the repo reads a different set "
                  "of environment variables than the file documents.\n"
                  "      Run: ops/scripts/generate-envrc-example.py",
                  file=sys.stderr)
            return 1
        n = body.count("\nexport ")
        print(f"ok: .envrc.example matches the {n} variables the repo reads")
        return 0

    with open(OUT, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(body)
    print(f"wrote {OUT} ({body.count(chr(10) + 'export ')} variables)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
