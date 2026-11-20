#!/usr/bin/env python3
"""The goatbuilder update gate: installed releases must update to the candidate.

A release is only safe to publish if the goatbuilder binaries people already run
can install it. Each one decides what it trusts from what was compiled into it,
so a change of release signer, surface layout or verification can strand every
earlier install, and nothing in the candidate's own tests would notice. This gate
runs the real earlier releases against the candidate before it is published:

  1. The candidate's signed files (goatbuilder-linux-amd64, its .sha256,
     SHA256SUMS, SHA256SUMS.cosign-bundle) are served from a local stand-in
     surface, http://127.0.0.1:<port>/goatbuilder, with a version.json naming
     the candidate: the layout get.<site>/goatbuilder/ serves.
  2. Each installed release runs `goatbuilder update` against it, from an empty
     directory, so no checkout configuration adds a signer: the binary's own
     compiled-in trust decides, as it does for anyone running it outside a
     checkout.
  3. It must report a signature it verified for this repository's release
     workflow, install the candidate, and then report the candidate's version.
     Anything else fails.

Which installed releases: the last release of each 0.x minor from the declared
`floor` up, and the last release of each later major, all below the candidate.
A release that cannot update and cannot be fixed in place is declared in
ops/ci/goatbuilder-update-gate.yaml with the code it refuses with, a one-line
reason, and the fix its error catalog entry gives; the gate runs it too and
asserts that refusal. Any other refusal fails the gate, and so does a declared
refusal that now installs (the list only shrinks).

A build that is not published in this repository (one built before the release
repository moved, say) cannot be run here. It is declared under
`measured_elsewhere` by the sha256 of its binary, with the refusal measured where
that binary is kept, the reason and the fix. The gate checks that list's shape,
not its result.

Rows print the version, the result and the code. They never print the
certificate subjects an update names, so no signer identity reaches a log.

usage:
  goatbuilder-update-gate.py select --candidate <version> [--declared FILE]
      the installed releases to run, one per line, from this repository's tags
  goatbuilder-update-gate.py run --candidate-dir DIR --candidate <version>
      --installed-dir DIR [--signer OWNER/REPO] [--declared FILE] [--negative-bundle FILE]
      DIR holds goatbuilder-linux-amd64 per installed version as <version>/goatbuilder;
      --negative-bundle serves another release's signature bundle, and every
      release must then refuse (the gate's negative control)
exit 0 every row as expected · 1 a row is not · 2 the inputs are wrong
"""
import argparse
import functools
import hashlib
import http.server
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
from dataclasses import dataclass
from pathlib import Path

import yaml

HERE = Path(__file__).resolve().parent
DECLARED = HERE / "goatbuilder-update-gate.yaml"
TAG_PREFIX = "goatbuilder/v"
ASSET = "goatbuilder-linux-amd64"
SIGNED = (ASSET, f"{ASSET}.sha256", "SHA256SUMS", "SHA256SUMS.cosign-bundle")
SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
CODE = re.compile(r"\b(GB-\d{3})\b")
VERSION = re.compile(r"\bgoatbuilder (\S+)")
UPDATE_TIMEOUT = 600
# A pass requires a reported signature verification, naming this repository.
SIGNED_BY = re.compile(r"\bsigned by\b(.*)")
SHA256 = re.compile(r"[0-9a-f]{64}")
# A measured_elsewhere entry describes a build by what it is, never where it came from.
NAMES_A_PLACE = re.compile(r"(https?://|github\.com|[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)")


class DeclaredError(ValueError):
    pass


class CandidateError(ValueError):
    pass


@dataclass(frozen=True)
class Refusal:
    code: str
    why: str
    fix: str


@dataclass(frozen=True)
class Elsewhere:
    version: str
    result: str
    why: str
    fix: str


@dataclass(frozen=True)
class Declared:
    floor: str
    refuses: dict
    elsewhere: dict


@dataclass(frozen=True)
class Result:
    outcome: str  # installed | unsigned (no verified signature reported) | refused | broken
    version: str | None
    code: str | None


def parse(v):
    m = SEMVER.match(v)
    if not m:
        raise ValueError(f"{v!r} is not a release version (major.minor.patch)")
    return tuple(int(x) for x in m.groups())


# ---- which installed releases -----------------------------------------------------------

def select_installed(tags, candidate, floor):
    """The last release of each 0.x minor from `floor`, and the last of each later major,
    below `candidate`. Pre-releases are never installed versions."""
    cand = parse(candidate)
    fl = tuple(int(x) for x in floor.split("."))
    last = {}
    for t in tags:
        if not t.startswith(TAG_PREFIX):
            continue
        v = t[len(TAG_PREFIX):]
        if not SEMVER.match(v):
            continue
        p = parse(v)
        if p >= cand:
            continue
        key = (0, p[1]) if p[0] == 0 else (p[0],)
        if p[0] == 0 and (0, p[1]) < fl:
            continue
        if key not in last or p > last[key]:
            last[key] = p
    return [".".join(map(str, p)) for p in sorted(last.values())]


def load_declared(path):
    data = yaml.safe_load(Path(path).read_text()) or {}
    floor = str(data.get("floor", "")).strip()
    if not re.match(r"^\d+\.\d+$", floor):
        raise DeclaredError(f"{path}: floor must be a major.minor, found {floor!r}")
    refuses = {}
    for i, e in enumerate(data.get("refuses") or []):
        for k in ("version", "code", "why", "fix"):
            if not str(e.get(k, "")).strip():
                raise DeclaredError(f"{path}: refuses[{i}] has no {k}")
        v, code = str(e["version"]), str(e["code"])
        parse(v)
        if not CODE.fullmatch(code):
            raise DeclaredError(f"{path}: refuses[{i}] code {code!r} is not a GB-NNN code")
        if code not in str(e["fix"]):
            raise DeclaredError(f"{path}: refuses[{i}] fix must name its code ({code}), so it points at that catalog entry")
        if v in refuses:
            raise DeclaredError(f"{path}: {v} is declared twice")
        refuses[v] = Refusal(code, str(e["why"]), str(e["fix"]))
    elsewhere = {}
    for i, e in enumerate(data.get("measured_elsewhere") or []):
        for k in ("version", "sha256", "result", "why", "fix"):
            if not str(e.get(k, "")).strip():
                raise DeclaredError(f"{path}: measured_elsewhere[{i}] has no {k}")
        sha, result = str(e["sha256"]).lower(), str(e["result"])
        parse(str(e["version"]))
        if not SHA256.fullmatch(sha):
            raise DeclaredError(f"{path}: measured_elsewhere[{i}] sha256 is not 64 hex digits")
        m = re.fullmatch(r"refused (GB-\d{3})", result)
        if not m:
            raise DeclaredError(f"{path}: measured_elsewhere[{i}] result must be 'refused GB-NNN' "
                                "(a build that installs needs no entry)")
        if m.group(1) not in str(e["fix"]):
            raise DeclaredError(f"{path}: measured_elsewhere[{i}] fix must name its code ({m.group(1)})")
        if any(NAMES_A_PLACE.search(str(e[k])) for k in ("why", "fix")):
            raise DeclaredError(f"{path}: measured_elsewhere[{i}] names a repository or a URL; say what the build is, not where")
        if sha in elsewhere:
            raise DeclaredError(f"{path}: measured_elsewhere sha256 {sha[:12]} is declared twice")
        elsewhere[sha] = Elsewhere(str(e["version"]), result, str(e["why"]), str(e["fix"]))
    return Declared(floor, refuses, elsewhere)


# ---- one run ------------------------------------------------------------------------------

def read_result(output, after_version, signer):
    """signer: owner/repository the candidate is released from. Only the outcome, version
    and code are kept, never the subject text."""
    m = VERSION.search(after_version or "")
    version = m.group(1) if m else None
    code = CODE.search(output or "")
    if code:
        return Result("refused", version, code.group(1))
    verified = any(signer and signer in s.group(1) for s in SIGNED_BY.finditer(output or ""))
    if version and not verified:
        return Result("unsigned", version, None)
    if version:
        return Result("installed", version, None)
    return Result("broken", None, None)


def judge(installed, r, candidate, declared):
    """(as expected, what was expected)."""
    if installed in declared:
        want = f"refused {declared[installed].code}"
        return (r.outcome == "refused" and r.code == declared[installed].code, want)
    want = f"installed {candidate}"
    return (r.outcome == "installed" and r.version == candidate, want)


def judge_negative(installed, r):
    """Under the negative control (another release's signature bundle is served), every
    release must refuse: an install means its signature check passed a wrong signature."""
    return (r.outcome == "refused", "refused")


def render_row(installed, r, want, ok):
    got = {"installed": f"installed {r.version}", "unsigned": f"no signature {r.version}",
           "refused": f"refused {r.code}"}.get(r.outcome, "no result")
    return f"{installed:<10} {got:<20} expected {want:<20} {'ok' if ok else 'FAIL'}"


# ---- the stand-in surface -----------------------------------------------------------------

def stage(candidate_dir, candidate, root, negative_bundle=None):
    candidate_dir = Path(candidate_dir)
    missing = [f for f in SIGNED if not (candidate_dir / f).is_file()]
    if missing:
        raise CandidateError(f"the candidate at {candidate_dir} has no {', '.join(missing)}")
    surface = Path(root) / "goatbuilder"
    surface.mkdir(parents=True, exist_ok=True)
    for f in SIGNED:
        shutil.copy2(candidate_dir / f, surface / f)
    if negative_bundle is not None:
        shutil.copy2(negative_bundle, surface / "SHA256SUMS.cosign-bundle")
    (surface / "version.json").write_text(json.dumps({"version": candidate}) + "\n")
    return surface


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a):
        pass


def serve(root):
    handler = functools.partial(Quiet, directory=str(root))
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv, f"http://127.0.0.1:{srv.server_address[1]}/goatbuilder"


def update_once(binary, url, work, signer):
    """Run one installed binary's update against `url`; return its Result."""
    home, empty, bindir = work / "home", work / "checkout", work / "bin"
    for d in (home, empty, bindir):
        d.mkdir(parents=True, exist_ok=True)
    exe = bindir / "goatbuilder"
    shutil.copy2(binary, exe)
    exe.chmod(0o755)
    env = {k: v for k, v in os.environ.items() if not k.startswith(("GIT_", "GOATBUILDER_"))}
    env.update(HOME=str(home), GOATBUILDER_UPDATE_URL=url)
    try:
        p = subprocess.run([str(exe), "update", "-C", str(empty)], cwd=empty, env=env,
                           capture_output=True, text=True, timeout=UPDATE_TIMEOUT)
        out = p.stdout + p.stderr
    except subprocess.TimeoutExpired:
        out = ""
    after = subprocess.run([str(exe), "--version"], env=env, capture_output=True, text=True).stdout
    return read_result(out, after, signer)


def digests(surface):
    return {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(Path(surface).iterdir())}


def run(candidate_dir, candidate, binaries, declared, work, negative_bundle=None, signer=None):
    """binaries: {installed version: path to that release's goatbuilder}. With
    negative_bundle, the candidate is served with that signature bundle instead of its
    own, and every release must refuse."""
    root = Path(work) / "surface"
    surface = stage(candidate_dir, candidate, root, negative_bundle)
    before = digests(surface)
    srv, url = serve(root)
    rows, failed = [], False
    try:
        for v in sorted(binaries, key=parse):
            r = update_once(Path(binaries[v]), url, Path(work) / v, signer)
            ok, want = judge_negative(v, r) if negative_bundle else judge(v, r, candidate, declared)
            failed |= not ok
            rows.append(render_row(v, r, want, ok))
    finally:
        srv.shutdown()
    changed = sorted(k for k, d in digests(surface).items() if before.get(k) != d)
    if changed:
        failed = True
        rows.append(f"the served files changed during the run: {', '.join(changed)}")
    return (1 if failed else 0), rows


# ---- command line -------------------------------------------------------------------------

def repo_tags():
    out = subprocess.run(["git", "tag", "-l", f"{TAG_PREFIX}*"], capture_output=True, text=True, check=True).stdout
    return out.split()


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("select")
    s.add_argument("--candidate", required=True)
    s.add_argument("--declared", default=str(DECLARED))
    r = sub.add_parser("run")
    r.add_argument("--candidate-dir", required=True)
    r.add_argument("--candidate", required=True)
    r.add_argument("--installed-dir", required=True)
    r.add_argument("--declared", default=str(DECLARED))
    r.add_argument("--signer", default=os.environ.get("GITHUB_REPOSITORY", ""),
                   help="owner/repository the candidate is released from (default: GITHUB_REPOSITORY)")
    r.add_argument("--negative-bundle", help="serve this signature bundle (another release's) instead "
                   "of the candidate's: every release must then refuse")
    a = ap.parse_args(argv)
    try:
        d = load_declared(a.declared)
        parse(a.candidate)
    except (DeclaredError, ValueError) as e:
        print(f"goatbuilder-update-gate: {e}", file=sys.stderr)
        return 2
    if a.cmd == "select":
        versions = sorted(set(select_installed(repo_tags(), a.candidate, d.floor)) |
                          {v for v in d.refuses if parse(v) < parse(a.candidate)}, key=parse)
        print("\n".join(versions))
        return 0
    if a.cmd == "run" and not a.signer:
        print("goatbuilder-update-gate: name the release repository with --signer (or GITHUB_REPOSITORY)", file=sys.stderr)
        return 2
    inst = Path(a.installed_dir)
    binaries = {p.name: p / "goatbuilder" for p in inst.iterdir() if (p / "goatbuilder").is_file()}
    if not binaries:
        print(f"goatbuilder-update-gate: no installed releases under {inst}", file=sys.stderr)
        return 2
    try:
        with tempfile.TemporaryDirectory() as work:
            rc, rows = run(a.candidate_dir, a.candidate, binaries, d.refuses, Path(work),
                           Path(a.negative_bundle) if a.negative_bundle else None, a.signer)
    except CandidateError as e:
        print(f"goatbuilder-update-gate: {e}", file=sys.stderr)
        return 2
    print(f"goatbuilder {a.candidate}: installed releases updating to it on a stand-in surface"
          + (" (negative control: another release's signature bundle)" if a.negative_bundle else ""))
    print("\n".join(rows))
    if a.negative_bundle:
        verdict = ("every installed release refused the candidate served with another release's signature"
                   if rc == 0 else "an installed release accepted a signature that is not the candidate's: "
                   "the release is not published")
    else:
        verdict = ("every installed release reaches the candidate as declared" if rc == 0
                   else "an installed release does not reach the candidate as declared: the release is not published")
    print("goatbuilder-update-gate: " + verdict)
    return rc


if __name__ == "__main__":
    sys.exit(main())
