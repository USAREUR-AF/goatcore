#!/usr/bin/env python3
"""check-reusable-workflow-permissions.py: a called workflow asks for no more than its caller grants.

WHY THIS EXISTS. GitHub refuses to start a run whose reusable workflow (a job
`uses: ./.github/workflows/<file>.yml`) declares a permission that the calling
job does not grant. The run fails at startup, before any job, with no log a
pull request ever shows: pull-request CI does not run the release callers, so a
change to the called workflow passes review and the tag's release then cannot
publish. goatbuilder/v0.5.2 stopped exactly so, when the docs job of
_release-binary.yml came to need `packages: read`.

THE RULE. For every job that calls a local reusable workflow, the permissions
that job has (its own `permissions:`, else its workflow's top-level ones, else
GitHub's default for the repository, treated here as granting nothing extra)
must cover every permission the called workflow asks for, at its top level and
in each of its jobs. `write` covers `read`; `read` covers only `read`.
`permissions: read-all` and `write-all` are expanded. A called workflow that
itself calls another is checked against the permissions it passes on.

Usage: check-reusable-workflow-permissions.py [--self-test]
Exits 1 naming each caller job, called file, scope, asked and granted level.
"""
import os
import sys
import tempfile

import yaml

LEVEL = {"none": 0, "read": 1, "write": 2}
SCOPES = [
    "actions", "attestations", "checks", "contents", "deployments", "discussions",
    "id-token", "issues", "models", "packages", "pages", "pull-requests",
    "repository-projects", "security-events", "statuses",
]


def norm(perms):
    """A permissions value as {scope: level}; None means 'not declared'."""
    if perms is None:
        return None
    if isinstance(perms, str):
        if perms == "read-all":
            return {s: 1 for s in SCOPES}
        if perms == "write-all":
            return {s: 2 for s in SCOPES}
        return {}
    if perms == {}:
        return {}
    return {k: LEVEL.get(str(v), 0) for k, v in perms.items()}


def load(path):
    with open(path) as f:
        return yaml.safe_load(f) or {}


def asked(wf):
    """Every scope a workflow file asks for, with the highest level asked."""
    want = {}
    for p in [wf.get("permissions")] + [
        (j or {}).get("permissions") for j in (wf.get("jobs") or {}).values()
    ]:
        for k, v in (norm(p) or {}).items():
            want[k] = max(want.get(k, 0), v)
    return want


def only_called(wf):
    """True when a workflow's only trigger is workflow_call (PyYAML reads `on:` as True)."""
    on = wf.get("on", wf.get(True))
    if isinstance(on, str):
        return on == "workflow_call"
    if isinstance(on, list):
        return on == ["workflow_call"]
    if isinstance(on, dict):
        return list(on.keys()) == ["workflow_call"]
    return False


def check(root):
    wfdir = os.path.join(root, ".github", "workflows")
    problems = []
    seen = set()

    def visit(path, granted_by_caller, chain):
        wf = load(path)
        top = norm(wf.get("permissions"))
        for name, job in (wf.get("jobs") or {}).items():
            job = job or {}
            uses = job.get("uses")
            if not (isinstance(uses, str) and uses.startswith("./.github/workflows/")):
                continue
            jp = norm(job.get("permissions"))
            if jp is not None:
                have = jp
            elif top is not None:
                have = top
            elif granted_by_caller is not None:
                # A called workflow's job with no block, in a file with no top level,
                # runs with what its caller passed.
                have = dict(granted_by_caller)
            else:
                have = {}
            if granted_by_caller is not None:
                # Inside a called workflow, a job can't hold more than the caller gave.
                have = {k: min(v, granted_by_caller.get(k, 0)) for k, v in have.items()}
            called = os.path.join(root, uses[2:].split("@")[0])
            if not os.path.exists(called):
                problems.append(f"{os.path.relpath(path, root)} job {name}: calls {uses}, which does not exist")
                continue
            for scope, lvl in sorted(asked(load(called)).items()):
                if lvl > have.get(scope, 0):
                    inv = {v: k for k, v in LEVEL.items()}
                    problems.append(
                        f"{os.path.relpath(path, root)} job {name} calls {uses[2:]}, which asks "
                        f"{scope}: {inv[lvl]}; the job grants {scope}: {inv[have.get(scope, 0)]}"
                    )
            key = (called, tuple(sorted(have.items())))
            if key not in seen and called not in chain:
                seen.add(key)
                visit(called, have, chain + [called])

    for fn in sorted(os.listdir(wfdir)):
        if not fn.endswith((".yml", ".yaml")):
            continue
        path = os.path.join(wfdir, fn)
        if only_called(load(path)):
            # Reached through its callers, with what they grant.
            continue
        visit(path, None, [path])
    return problems


def run_fixture(files):
    with tempfile.TemporaryDirectory() as d:
        os.makedirs(os.path.join(d, ".github", "workflows"))
        for name, text in files.items():
            with open(os.path.join(d, ".github/workflows", name), "w") as f:
                f.write(text)
        return check(d)


def self_test():
    leaf = ("on: workflow_call\njobs:\n  x:\n    permissions:\n      packages: read\n"
            "    runs-on: x\n    steps: []\n")
    cases = [
        # (name, files, refused?)
        ("a caller job's block replaces its workflow's top level", {
            "caller.yml": "on: push\npermissions:\n  packages: read\njobs:\n  a:\n"
                          "    permissions:\n      contents: read\n    uses: ./.github/workflows/_leaf.yml\n",
            "_leaf.yml": leaf}, True),
        ("permissions: {} grants nothing", {
            "caller.yml": "on: push\njobs:\n  a:\n    permissions: {}\n    uses: ./.github/workflows/_leaf.yml\n",
            "_leaf.yml": leaf}, True),
        ("a nested call with no block inherits what the outer caller passed", {
            "caller.yml": "on: push\njobs:\n  a:\n    permissions:\n      packages: read\n"
                          "    uses: ./.github/workflows/_mid.yml\n",
            "_mid.yml": "on: workflow_call\njobs:\n  b:\n    uses: ./.github/workflows/_leaf.yml\n",
            "_leaf.yml": leaf}, False),
    ]
    for name, files, refused in cases:
        got = run_fixture(files)
        if bool(got) != refused:
            print(f"SELF-TEST FAILED: {name}: expected {'a refusal' if refused else 'a pass'}, got {got}")
            return 1
    caller = (
        "on: push\npermissions:\n  contents: read\njobs:\n  release:\n"
        "    permissions:\n      contents: write\n      id-token: write\n"
        "    uses: ./.github/workflows/_called.yml\n"
    )
    called_bad = (
        "on: workflow_call\npermissions:\n  contents: read\njobs:\n  docs:\n"
        "    permissions:\n      contents: read\n      packages: read\n    runs-on: x\n    steps: []\n"
    )
    called_ok = called_bad.replace("      packages: read\n", "")
    for text, want in ((called_bad, 1), (called_ok, 0)):
        with tempfile.TemporaryDirectory() as d:
            os.makedirs(os.path.join(d, ".github", "workflows"))
            open(os.path.join(d, ".github/workflows/caller.yml"), "w").write(caller)
            open(os.path.join(d, ".github/workflows/_called.yml"), "w").write(text)
            got = check(d)
            if (len(got) > 0) != bool(want):
                print("SELF-TEST FAILED:", "passed a called workflow asking packages: read" if want else f"refused a covered call: {got}")
                return 1
    print("check-reusable-workflow-permissions: self-test ok")
    return 0


def main():
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
    problems = check(root)
    if problems:
        print("check-reusable-workflow-permissions: a called workflow asks for more than its caller grants.")
        print("GitHub refuses such a run at startup, before any job runs:")
        for p in problems:
            print("  " + p)
        print("Grant the scope on the calling job, or remove it from the called workflow.")
        print("(A caller that declares no permissions is read here as granting none: declare them on the calling job.)")
        return 1
    print("check-reusable-workflow-permissions: ok, every called workflow is covered by its caller")
    return 0


if __name__ == "__main__":
    sys.exit(main())
