#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""check-expected-workflows-ran.py: a pull request has a run of every workflow its change triggers on main.

WHY THIS EXISTS. Issue #741. A workflow whose `on.pull_request` filter names
`branches: [main]` does not run for a pull request based on another pull
request's branch. When the lower pull request merges, GitHub retargets the
upper one to main with an `edited` event, which no workflow listens for, so the
upper one can merge without that workflow ever having run on its code. Only a
few checks are required on main, so nothing else catches it.

WHAT IT CHECKS. It reads every workflow's `on.pull_request` and
`on.pull_request_target` filters (`branches`, `branches-ignore`, `paths`,
`paths-ignore`, `types`) and evaluates them against the pull request's changed
files as if its base were main. That gives the workflows the change should have
run. A workflow that the changed files do not trigger is not expected, so a
paths filter never causes a refusal. Each expected workflow needs a completed
run, from a pull_request or pull_request_target event, on the pull request's
head commit; it is matched by workflow file path. The check refuses and names
every expected workflow that has none. It does not judge whether a run passed;
the merge rules do that.

The filters come from the base branch (default `origin/main` in this checkout,
so fetch first), not from the pull request: a pull request that edits a
workflow's triggers is judged by main's version.

WHO RUNS IT. Whoever merges, before merging:
  ops/ci/check-expected-workflows-ran.py 741
CI runs only --self-test and --tree; a verdict on a pull request needs that
pull request's other workflows to have finished.

Usage:
  ops/ci/check-expected-workflows-ran.py <pr> [--repo OWNER/NAME] [--ref REF | --workflows DIR]
  ops/ci/check-expected-workflows-ran.py --expected FILE... [--base BRANCH] [--ref REF | --workflows DIR]
  ops/ci/check-expected-workflows-ran.py --tree [--workflows DIR]
  ops/ci/check-expected-workflows-ran.py --self-test

  <pr>        refuse (exit 1) when an expected workflow has no completed run on the head
  --expected  print the workflows these changed files trigger (no network)
  --tree      refuse a pull_request trigger limited to main by `branches`, unless the
              workflow file says why on a line containing `expected-workflows: main-only`
Needs PyYAML, and `gh` (signed in) for <pr>. Exit 2 on a usage or read error.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tempfile

try:
    import yaml
except ImportError:  # pragma: no cover
    print("needs PyYAML: pip install pyyaml", file=sys.stderr)
    sys.exit(2)

WORKFLOW_DIR = ".github/workflows"
PR_EVENTS = ("pull_request", "pull_request_target")
# The events GitHub fires when a pull request's head commit changes or it opens.
HEAD_TYPES = {"opened", "synchronize", "reopened"}
KNOWN_FILTERS = {"branches", "branches-ignore", "paths", "paths-ignore", "types"}
MAIN_ONLY_MARK = "expected-workflows: main-only"
# GitHub evaluates path filters against at most this many changed files.
GITHUB_DIFF_LIMIT = 300


class ReadError(Exception):
    pass


# ---------------------------------------------------------------- filters


def glob_re(pat: str) -> re.Pattern:
    """GitHub's filter pattern syntax, as a regular expression over the whole string.

    `*` is any run without `/`; `**` is any run; `**/` also matches no directory;
    `?` and `+` quantify the preceding character; `[..]` is a class; a backslash
    escapes the next character.
    """
    out, i = [], 0
    while i < len(pat):
        c = pat[i]
        if c == "*":
            if pat[i:i + 3] == "**/":
                out.append("(?:.*/)?")
                i += 3
                continue
            if pat[i:i + 2] == "**":
                out.append(".*")
                i += 2
                continue
            out.append("[^/]*")
        elif c in "?+":
            out.append(c)
        elif c == "[":
            j = pat.find("]", i + 1)
            if j < 0:
                out.append(re.escape(c))
            else:
                out.append(pat[i:j + 1])
                i = j + 1
                continue
        elif c == "\\" and i + 1 < len(pat):
            out.append(re.escape(pat[i + 1]))
            i += 2
            continue
        else:
            out.append(re.escape(c))
        i += 1
    return re.compile("".join(out) + r"\Z")


def matches(patterns: list, s: str) -> bool:
    """The last matching pattern decides; a `!` pattern excludes."""
    hit = False
    for p in patterns:
        p = str(p)
        if p.startswith("!"):
            if glob_re(p[1:]).match(s):
                hit = False
        elif glob_re(p).match(s):
            hit = True
    return hit


def as_list(v) -> list:
    if v is None:
        return []
    return [v] if isinstance(v, str) else list(v)


def triggers(doc: dict) -> dict:
    on = doc.get("on")
    if on is None:
        on = doc.get(True)  # YAML 1.1 reads a bare `on:` key as boolean True
    if on is None:
        return {}
    if isinstance(on, str):
        return {on: None}
    if isinstance(on, list):
        return {k: None for k in on}
    if isinstance(on, dict):
        return dict(on)
    raise ReadError(f"unreadable `on:` {on!r}")


def branches_allow(spec: dict, base: str) -> bool:
    if "branches" in spec and not matches(as_list(spec["branches"]), base):
        return False
    if "branches-ignore" in spec and matches(as_list(spec["branches-ignore"]), base):
        return False
    return True


def event_runs(spec, base: str, files: list[str]) -> bool:
    """Would this pull_request / pull_request_target trigger run for a head push into `base`?"""
    if spec is None:
        spec = {}
    if not isinstance(spec, dict):
        raise ReadError(f"unreadable trigger {spec!r}")
    unknown = set(spec) - KNOWN_FILTERS
    if unknown:
        raise ReadError(f"unknown trigger filter {sorted(unknown)}")
    types = as_list(spec.get("types"))
    if types and not set(types) & HEAD_TYPES:
        return False
    if not branches_allow(spec, base):
        return False
    if "paths" in spec and not any(matches(as_list(spec["paths"]), f) for f in files):
        return False
    if "paths-ignore" in spec and all(matches(as_list(spec["paths-ignore"]), f) for f in files):
        return False
    return True


# ---------------------------------------------------------------- workflows


def load_from_dir(d: str) -> dict[str, str]:
    out = {}
    for name in sorted(os.listdir(d)):
        if name.endswith((".yml", ".yaml")) and os.path.isfile(os.path.join(d, name)):
            with open(os.path.join(d, name), encoding="utf-8") as fh:
                out[f"{WORKFLOW_DIR}/{name}"] = fh.read()
    return out


def load_from_ref(ref: str) -> dict[str, str]:
    def git(*args: str) -> str:
        r = subprocess.run(["git", *args], capture_output=True, text=True)
        if r.returncode != 0:
            raise ReadError(f"git {' '.join(args)}: {r.stderr.strip()}")
        return r.stdout

    out = {}
    for name in git("ls-tree", "--name-only", f"{ref}:{WORKFLOW_DIR}/").split("\n"):
        if name.endswith((".yml", ".yaml")):
            out[f"{WORKFLOW_DIR}/{name}"] = git("show", f"{ref}:{WORKFLOW_DIR}/{name}")
    return out


def parse(sources: dict[str, str]) -> dict[str, dict]:
    docs = {}
    for path, text in sources.items():
        try:
            doc = yaml.safe_load(text) or {}
        except yaml.YAMLError as exc:
            raise ReadError(f"{path}: does not parse as YAML ({exc})") from exc
        if not isinstance(doc, dict):
            raise ReadError(f"{path}: not a mapping")
        docs[path] = doc
    return docs


def expected_workflows(docs: dict[str, dict], base: str, files: list[str]) -> list[str]:
    """Paths of the workflows a pull request into `base` changing `files` runs."""
    out = []
    for path, doc in docs.items():
        try:
            on = triggers(doc)
            if any(ev in on and event_runs(on[ev], base, files) for ev in PR_EVENTS):
                out.append(path)
        except ReadError as exc:
            raise ReadError(f"{path}: {exc}") from exc
    return sorted(out)


def missing_runs(expected: list[str], runs: list[dict]) -> list[str]:
    """Expected workflow paths with no completed pull-request run among `runs` (Actions API rows)."""
    have = {r.get("path") for r in runs
            if r.get("event") in PR_EVENTS and r.get("status") == "completed"}
    return [p for p in expected if p not in have]


def tree_problems(sources: dict[str, str], docs: dict[str, dict]) -> list[str]:
    problems = []
    for path, doc in docs.items():
        on = triggers(doc)
        for ev in PR_EVENTS:
            spec = on.get(ev)
            if not isinstance(spec, dict):
                continue
            if "branches" not in spec and "branches-ignore" not in spec:
                continue
            # Limited to main: it runs for a pull request into main and not for one based on another branch.
            if branches_allow(spec, "main") and not branches_allow(spec, "stacked/branch"):
                if MAIN_ONLY_MARK not in sources[path]:
                    problems.append(
                        f"{path}: {ev} runs only for pull requests into main (`branches`), so a pull request "
                        f"based on another branch skips it (#741); drop the branch filter, or add a comment "
                        f"`# {MAIN_ONLY_MARK}: <why>`"
                    )
    return problems


def repo_root() -> str:
    r = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True)
    if r.returncode != 0:
        raise ReadError("not inside a git checkout; pass --workflows DIR")
    return r.stdout.strip()


# ---------------------------------------------------------------- GitHub


def gh_json(path: str, paginate: bool = False):
    cmd = ["gh", "api", path] + (["--paginate", "--slurp"] if paginate else [])
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        raise ReadError(f"gh api {path}: {r.stderr.strip()}")
    return json.loads(r.stdout)


def repo_name() -> str:
    r = subprocess.run(["gh", "repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"],
                       capture_output=True, text=True)
    if r.returncode != 0 or not r.stdout.strip():
        raise ReadError("cannot tell the repository; pass --repo OWNER/NAME")
    return r.stdout.strip()


def check_pr(pr: str, repo: str, docs: dict[str, dict], base: str) -> int:
    info = gh_json(f"repos/{repo}/pulls/{pr}")
    head = info["head"]["sha"]
    pages = gh_json(f"repos/{repo}/pulls/{pr}/files?per_page=100", paginate=True)
    files = [f["filename"] for page in pages for f in page]
    if not files:
        raise ReadError(f"#{pr}: no changed files returned")
    pages = gh_json(f"repos/{repo}/actions/runs?head_sha={head}&per_page=100", paginate=True)
    runs = [r for page in pages for r in page.get("workflow_runs", [])]

    expected = expected_workflows(docs, base, files)
    missing = missing_runs(expected, runs)
    print(f"#{pr} head {head[:12]}: {len(files)} changed files trigger {len(expected)} workflows on {base}"
          f" (base now: {info['base']['ref']})")
    if len(files) > GITHUB_DIFF_LIMIT:
        print(f"  note: over {GITHUB_DIFF_LIMIT} changed files; GitHub may have skipped a path-filtered "
              f"workflow this check expects")
    if missing:
        for p in missing:
            print(f"  REFUSED: {p} has no completed run on {head[:12]}")
        print("  Push a new commit (or update the branch from main) so the missing workflows run on the head.")
        return 1
    print("  every expected workflow has a completed run on the head")
    return 0


# ---------------------------------------------------------------- self-test


SELF_TEST_WORKFLOWS = {
    "mermaid-renders.yml": """\
name: mermaid-renders
on:
  pull_request:
    branches: [main]
    paths: ['**/*.md', '.github/workflows/mermaid-renders.yml']
jobs: {render: {runs-on: x, steps: [{run: 'true'}]}}
""",
    "mermaid-fixed.yml": """\
name: mermaid-fixed
on:
  pull_request:
    paths: ['**/*.md']
jobs: {render: {runs-on: x, steps: [{run: 'true'}]}}
""",
    "ci.yml": """\
name: CI
on: [push, pull_request]
jobs: {test: {runs-on: x, steps: [{run: 'true'}]}}
""",
    "leak-gate.yml": """\
name: leak-gate
on:
  pull_request_target:
    types: [opened, synchronize, reopened, ready_for_review]
jobs: {scan: {runs-on: x, steps: [{run: 'true'}]}}
""",
    "stable-guard.yml": """\
name: stable-guard
on:
  pull_request_target:
    branches: [stable]
    types: [opened, reopened]
jobs: {g: {runs-on: x, steps: [{run: 'true'}]}}
""",
    "labels-only.yml": """\
name: labels-only
on:
  pull_request:
    types: [labeled]
jobs: {g: {runs-on: x, steps: [{run: 'true'}]}}
""",
    "not-docs.yml": """\
name: not-docs
on:
  pull_request:
    paths-ignore: ['docs/**', '!docs/keep/**']
jobs: {g: {runs-on: x, steps: [{run: 'true'}]}}
""",
    "rust.yml": """\
name: rust
on:
  pull_request:
    paths: ['**/Cargo.lock', '**/Cargo.toml']
  schedule: [{cron: '0 3 * * 0'}]
jobs: {g: {runs-on: x, steps: [{run: 'true'}]}}
""",
    "reusable.yml": """\
name: reusable
on: {workflow_call: {}}
jobs: {g: {runs-on: x, steps: [{run: 'true'}]}}
""",
}


def run(path: str, event: str = "pull_request", status: str = "completed") -> dict:
    return {"path": f"{WORKFLOW_DIR}/{path}", "event": event, "status": status}


def self_test() -> int:
    failures = []

    def expect(label, got, want):
        if got != want:
            failures.append(f"{label}: got {got!r}, want {want!r}")

    globs = [
        ("**.go", "a/b/c.go", True), ("**.go", "c.go", True), ("docs/**", "docs/a/b.md", True),
        ("docs/*.md", "docs/a/b.md", False), ("docs/*.md", "docs/b.md", True),
        ("**/*.md", "README.md", True), ("**/*.md", "a/b/README.md", True),
        ("**/Dockerfile", "ops/x/Dockerfile", True), ("**/Dockerfile", "Dockerfile.x", False),
        ("ops/ansible/roles/*-oidc-client/**", "ops/ansible/roles/x-oidc-client/t/a", True),
        ("*", "a/b", False), ("ops/ansible/*.yml", "ops/ansible/roles/a.yml", False),
        ("**/Dockerfile*", "a/Dockerfile.dev", True), ("v[12].txt", "v2.txt", True),
        ("main", "main", True), ("main", "mainline", False), ("release/**", "release/0.7", True),
    ]
    for pat, s, want in globs:
        expect(f"glob {pat} ~ {s}", bool(glob_re(pat).match(s)), want)
    expect("negation excludes", matches(["docs/**", "!docs/x/**"], "docs/x/a"), False)
    expect("negation keeps others", matches(["docs/**", "!docs/x/**"], "docs/y/a"), True)

    with tempfile.TemporaryDirectory() as tmp:
        for name, text in SELF_TEST_WORKFLOWS.items():
            with open(os.path.join(tmp, name), "w", encoding="utf-8") as fh:
                fh.write(text)
        sources = load_from_dir(tmp)
        docs = parse(sources)

    def wf(*names):
        return sorted(f"{WORKFLOW_DIR}/{n}" for n in names)

    md = ["docs/design/x.md"]
    expect("a *.md change into main", expected_workflows(docs, "main", md),
           wf("ci.yml", "leak-gate.yml", "mermaid-fixed.yml", "mermaid-renders.yml"))
    expect("a *.md change into a stacked branch skips the main-only workflow",
           expected_workflows(docs, "feat/lower", md),
           wf("ci.yml", "leak-gate.yml", "mermaid-fixed.yml"))
    expect("paths-ignore with a ! re-include",
           expected_workflows(docs, "main", ["docs/keep/a.txt"]),
           wf("ci.yml", "leak-gate.yml", "not-docs.yml"))
    expect("a change no filter matches expects only the unfiltered workflows",
           expected_workflows(docs, "main", ["ops/somewhere/else.sh"]),
           wf("ci.yml", "leak-gate.yml", "not-docs.yml"))
    expect("a Cargo.lock change", expected_workflows(docs, "main", ["crates/Cargo.lock"]),
           wf("ci.yml", "leak-gate.yml", "not-docs.yml", "rust.yml"))

    # The acceptance cases from #741.
    stacked_runs = [run("ci.yml"), run("leak-gate.yml", "pull_request_target"), run("mermaid-fixed.yml")]
    expect("stacked pull request touching *.md lacks the render workflow: refused, named",
           missing_runs(expected_workflows(docs, "main", md), stacked_runs), wf("mermaid-renders.yml"))
    into_main_runs = stacked_runs + [run("mermaid-renders.yml")]
    expect("pull request into main that has the render run: passes",
           missing_runs(expected_workflows(docs, "main", md), into_main_runs), [])
    expect("a change no filter matches: passes with the unfiltered runs",
           missing_runs(expected_workflows(docs, "main", ["ops/x.sh"]),
                        [run("ci.yml"), run("leak-gate.yml", "pull_request_target"), run("not-docs.yml")]), [])
    expect("a run still in progress does not count",
           missing_runs(wf("ci.yml"), [run("ci.yml", status="in_progress")]), wf("ci.yml"))
    expect("a push run does not count",
           missing_runs(wf("ci.yml"), [run("ci.yml", event="push")]), wf("ci.yml"))

    # --tree: the main-only workflow is refused; the marked one is not.
    tree = tree_problems(sources, docs)
    expect("tree names the main-only workflow", [p.split(":")[0] for p in tree], wf("mermaid-renders.yml"))
    marked = dict(sources)
    marked[f"{WORKFLOW_DIR}/mermaid-renders.yml"] = (
        f"# {MAIN_ONLY_MARK}: fixture\n" + marked[f"{WORKFLOW_DIR}/mermaid-renders.yml"])
    expect("a marked main-only workflow passes --tree", tree_problems(marked, parse(marked)), [])

    # An unknown filter key is an error, never a silent pass.
    try:
        event_runs({"paths": ["a"], "tags": ["v*"]}, "main", ["a"])
        failures.append("unknown filter key: no error")
    except ReadError:
        pass

    for f in failures:
        print(f"self-test FAIL: {f}")
    if failures:
        return 1
    print("self-test ok")
    return 0


# ---------------------------------------------------------------- main


def main(argv: list[str]) -> int:
    args = list(argv)
    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        return 0 if args else 2
    if args == ["--self-test"]:
        return self_test()

    def opt(name: str, default=None):
        if name in args:
            i = args.index(name)
            if i + 1 >= len(args):
                raise ReadError(f"{name} needs a value")
            v = args[i + 1]
            del args[i:i + 2]
            return v
        return default

    try:
        repo = opt("--repo")
        ref = opt("--ref")
        wdir = opt("--workflows")
        base = opt("--base", "main")
        tree = bool(args) and args[0] == "--tree"
        if wdir:
            sources = load_from_dir(wdir)
        elif tree and not ref:
            sources = load_from_dir(os.path.join(repo_root(), WORKFLOW_DIR))  # --tree judges this checkout
        else:
            sources = load_from_ref(ref or "origin/main")
        docs = parse(sources)
        if tree:
            problems = tree_problems(sources, docs)
            for p in problems:
                print(f"REFUSED: {p}")
            if not problems:
                print(f"no pull_request trigger is limited to main ({len(docs)} workflows)")
            return 1 if problems else 0
        if args and args[0] == "--expected":
            files = args[1:]
            if not files:
                raise ReadError("--expected needs at least one file")
            for p in expected_workflows(docs, base, files):
                print(p)
            return 0
        if len(args) != 1 or not args[0].isdigit():
            raise ReadError("give one pull request number")
        return check_pr(args[0], repo or repo_name(), docs, base)
    except ReadError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
