#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# Does a module's test suite actually RUN anywhere?
#
# THE FAILURE THIS EXISTS FOR. F-417: ops/goat-id-console carried *_test.go
# files that no workflow ever executed — the tests existed, passed locally, and
# gave everyone the feeling the module was covered, while CI ran them exactly
# zero times. A regression could land green. It was not unique: a sweep of the
# tree found ~20 modules in the same state — a compiled, testable
# unit whose suite is wired into no `go test` / `cargo test` / `cargo nextest`
# step in any .github/workflows/ file.
#
# This guard does NOT fix those 20. Fixing them is per-module work (each needs
# a real workflow step, sometimes CGO deps or a service container). What this
# guard does is make the gap VISIBLE and NON-GROWING: the current unwired set
# is frozen in ops/ci/test-coverage-wired-allowlist.txt, and a NEW module that
# ships tests with no workflow to run them fails this guard on its own PR. The
# allowlist is shrink-only — wire a module up and its line must be removed or
# the guard fails ("you wired it — remove this line").
#
# WHAT COUNTS AS "WIRED". A workflow step whose `run:` contains go test /
# cargo test / cargo nextest, and whose command or working-directory NAMES the
# module — by directory path (Go working-directory, `cd`), by cargo package
# (`-p <name>` / `--package <name>`), or by testing the whole workspace the
# crate belongs to (`cargo test --workspace`/`--all` from the workspace root).
# `--all-features` is NOT `--all`. govulncheck / clippy / build steps do NOT
# count — only steps that actually execute tests.
#
# Usage:
#   check-test-coverage-wired.py                 # gate the tree, honor allowlist
#   check-test-coverage-wired.py --list          # show every module + verdict
#   check-test-coverage-wired.py --self-test     # prove red-first, then green
#
# Exit 0 = every test-bearing module is either wired or on the shrink-only
#          allowlist, and no allowlisted module has since been wired.
# Exit 1 = a test-bearing module runs nowhere and is not allowlisted, OR an
#          allowlisted module is now wired (remove its stale line).

import os
import re
import sys

try:
    import yaml
except ImportError:
    print("FAIL: PyYAML required (pip install pyyaml)", file=sys.stderr)
    sys.exit(2)

SKIP_DIR_PARTS = ("/.claude/worktrees/", "/vendor/", "/target/", "/node_modules/")
TEST_CMD_RE = re.compile(r"\b(go\s+test|cargo\s+test|cargo\s+nextest)\b")
CARGO_TEST_RE = re.compile(r"\bcargo\s+(?:test|nextest)\b")
PKG_ARG_RE = re.compile(r"(?:-p|--package)[= ]+([A-Za-z0-9_-]+)")
# A step that loops over every tracked go.mod (ci.yml: `for gm in $(git
# ls-files '*go.mod' ...)`) and runs `go test` in each tests EVERY Go module,
# minus the ones it skips by name (`[ "$gm" = <path> ] && continue`).
GO_ALL_MODULES_RE = re.compile(r"""git\s+ls-files\s+['"]?(?:\*\*/)?\*/?go\.mod['"]?""")
GO_LOOP_SKIP_RE = re.compile(r"""\[\s*"\$\w+"\s*=\s*['"]?([\w./-]+)['"]?\s*\]\s*&&\s*continue""")


def repo_root():
    import subprocess

    try:
        return subprocess.check_output(
            ["git", "rev-parse", "--show-toplevel"], text=True
        ).strip()
    except Exception:
        return os.getcwd()


def skip(path, root):
    # Compare RELATIVE to root: the guard's own checkout may itself live under
    # a path like .claude/worktrees/... (isolation worktree), which must not
    # make the whole tree invisible. Only nested skip-dirs matter.
    rel = os.path.relpath(path, root)
    p = "/" + rel.replace(os.sep, "/").strip("/") + "/"
    return any(part in p for part in SKIP_DIR_PARTS)


# ---------------------------------------------------------------------------
# 1. Enumerate modules
# ---------------------------------------------------------------------------
def find_modules(root):
    """Return (go_dirs, rust_dirs) as repo-relative POSIX paths.

    Rust: only Cargo.toml manifests that declare a [package] table — a
    workspace-root-only manifest ([workspace] with no [package]) is NOT a
    crate and is skipped."""
    go_dirs, rust_dirs = [], []
    for dirpath, dirnames, filenames in os.walk(root):
        rel = os.path.relpath(dirpath, root)
        if skip(dirpath, root):
            dirnames[:] = []
            continue
        # prune noisy trees early
        dirnames[:] = [
            d
            for d in dirnames
            if d not in (".git", "target", "vendor", "node_modules")
        ]
        if "go.mod" in filenames:
            go_dirs.append("." if rel == "." else rel.replace(os.sep, "/"))
        if "Cargo.toml" in filenames:
            try:
                with open(os.path.join(dirpath, "Cargo.toml"), encoding="utf-8") as fh:
                    txt = fh.read()
            except OSError:
                txt = ""
            if re.search(r"(?m)^\[package\]", txt):
                rust_dirs.append("." if rel == "." else rel.replace(os.sep, "/"))
    return sorted(go_dirs), sorted(rust_dirs)


def nearest_module(path, module_dirs):
    """Longest module-dir prefix that owns `path` (repo-relative)."""
    best = None
    for m in module_dirs:
        base = "" if m == "." else m + "/"
        if path == m or path.startswith(base):
            if best is None or len(m) > len(best):
                best = m
    return best


# ---------------------------------------------------------------------------
# 2. Detect tests, attributed to the nearest enclosing module
# ---------------------------------------------------------------------------
RUST_TEST_RE = re.compile(r"#\[\s*(?:tokio::)?test\b")


def modules_with_tests(root, go_dirs, rust_dirs):
    go_has, rust_has = set(), set()
    for dirpath, dirnames, filenames in os.walk(root):
        if skip(dirpath, root):
            dirnames[:] = []
            continue
        dirnames[:] = [
            d
            for d in dirnames
            if d not in (".git", "target", "vendor", "node_modules")
        ]
        for fn in filenames:
            full = os.path.join(dirpath, fn)
            rel = os.path.relpath(full, root).replace(os.sep, "/")
            if fn.endswith("_test.go"):
                owner = nearest_module(rel, go_dirs)
                if owner is not None:
                    go_has.add(owner)
            elif fn.endswith(".rs"):
                owner = nearest_module(rel, rust_dirs)
                if owner is None:
                    continue
                if owner in rust_has:
                    continue
                try:
                    with open(full, encoding="utf-8", errors="ignore") as fh:
                        if RUST_TEST_RE.search(fh.read()):
                            rust_has.add(owner)
                except OSError:
                    pass  # unreadable file: skip
    return go_has, rust_has


# ---------------------------------------------------------------------------
# 3. Harvest test-running steps from every workflow
# ---------------------------------------------------------------------------
def harvest_test_steps(wf_dir):
    """Return (contexts, workspace_tested_roots, packages).

    contexts: list of strings, each = working-directory + run text of one step
              that actually runs tests.
    dir_tested_roots: working-directories from which an UNRESTRICTED cargo test
              ran — bare `cargo test` or `cargo test --workspace`. From a
              workspace root that tests every member crate under it, so any
              crate under such a dir is wired (e.g. grant-crypto.yml runs bare
              `cargo test` from ops/grant-crypto/sdk, testing core + capi). A
              `cargo test -p NAME` is RESTRICTED and does not root-credit.
              (Go's `go test ./...` never crosses a nested go.mod, so a Go dir
              is NOT root-credited — its path token match stands alone.)
    packages: cargo package names named by `-p`/`--package` in a test step."""
    contexts, dir_roots, packages = [], set(), set()
    go_all = {"seen": False, "skips": set()}
    harvest_test_steps.go_all = go_all
    if not os.path.isdir(wf_dir):
        return contexts, dir_roots, packages
    for fn in sorted(os.listdir(wf_dir)):
        if not fn.endswith((".yml", ".yaml")):
            continue
        path = os.path.join(wf_dir, fn)
        try:
            with open(path, encoding="utf-8") as fh:
                doc = yaml.safe_load(fh)
        except Exception as e:
            print(f"  WARN: cannot parse {fn}: {e} — skipping", file=sys.stderr)
            continue
        if not isinstance(doc, dict):
            continue
        jobs = doc.get("jobs") or {}
        if not isinstance(jobs, dict):
            continue
        for job in jobs.values():
            if not isinstance(job, dict):
                continue
            job_wd = ""
            try:
                job_wd = job["defaults"]["run"]["working-directory"] or ""
            except (KeyError, TypeError):
                job_wd = ""
            for step in job.get("steps") or []:
                if not isinstance(step, dict):
                    continue
                run = step.get("run")
                if not isinstance(run, str) or not TEST_CMD_RE.search(run):
                    continue
                wd = step.get("working-directory") or job_wd or ""
                wd = str(wd).strip()
                # A module named only to be SKIPPED is not wired by that name.
                ctx = wd + "\n" + GO_LOOP_SKIP_RE.sub("", run)
                contexts.append(ctx)
                named = [m.group(1) for m in PKG_ARG_RE.finditer(run)]
                for n in named:
                    packages.add(n)
                if re.search(r"\bgo\s+test\b", run) and GO_ALL_MODULES_RE.search(run):
                    go_all["seen"] = True
                    go_all["skips"].update(GO_LOOP_SKIP_RE.findall(run))
                # A cargo test with no `-p`/`--package` tests the whole
                # workspace/package rooted at the working-directory.
                if CARGO_TEST_RE.search(run) and not named and wd:
                    dir_roots.add(wd.strip("/"))
    return contexts, dir_roots, packages


def token_in(needle, haystack):
    """needle appears in haystack bounded by non path/name chars."""
    for m in re.finditer(re.escape(needle), haystack):
        b = m.start() - 1
        a = m.end()
        before = haystack[b] if b >= 0 else ""
        after = haystack[a] if a < len(haystack) else ""
        if before and re.match(r"[\w/-]", before):
            continue
        if after and re.match(r"[\w/-]", after):
            continue
        return True
    return False


def pkg_name(root, crate_dir):
    p = os.path.join(root, crate_dir, "Cargo.toml")
    try:
        with open(p, encoding="utf-8") as fh:
            for line in fh:
                m = re.match(r'\s*name\s*=\s*"([^"]+)"', line)
                if m:
                    return m.group(1)
    except OSError:
        pass  # no or unreadable manifest: skip
    return os.path.basename(crate_dir)


def is_wired(kind, mdir, root, contexts, dir_roots, packages, go_all=None):
    joined = "\n".join(contexts)
    if kind == "go" and go_all and go_all["seen"] and mdir not in go_all["skips"]:
        return True
    if token_in(mdir, joined):
        return True
    if kind == "rust":
        name = pkg_name(root, mdir)
        if name in packages or token_in(name, joined):
            return True
        # A crate physically under a dir where an unrestricted cargo test ran
        # is tested as a workspace member of that root.
        for wr in dir_roots:
            if mdir == wr or mdir.startswith(wr + "/"):
                return True
    return False


# ---------------------------------------------------------------------------
# Allowlist (shrink-only)
# ---------------------------------------------------------------------------
def load_allowlist(path):
    entries = set()
    if path and os.path.exists(path):
        with open(path, encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if line and not line.startswith("#"):
                    entries.add(line)
    return entries


def compute(root):
    go_dirs, rust_dirs = find_modules(root)
    go_has, rust_has = modules_with_tests(root, go_dirs, rust_dirs)
    contexts, dir_roots, packages = harvest_test_steps(
        os.path.join(root, ".github", "workflows")
    )
    modules = [("go", d) for d in go_dirs if d in go_has] + [
        ("rust", d) for d in rust_dirs if d in rust_has
    ]
    unwired = []
    wired = []
    for kind, mdir in sorted(modules):
        if is_wired(kind, mdir, root, contexts, dir_roots, packages,
                    getattr(harvest_test_steps, "go_all", None)):
            wired.append((kind, mdir))
        else:
            unwired.append((kind, mdir))
    return wired, unwired, (go_dirs, rust_dirs, go_has, rust_has)


def main():
    args = sys.argv[1:]
    root = repo_root()
    allow_path = os.environ.get(
        "TEST_COVERAGE_ALLOWLIST",
        os.path.join(root, "ops", "ci", "test-coverage-wired-allowlist.txt"),
    )

    if "--self-test" in args:
        return self_test(root)

    wired, unwired, _ = compute(root)
    allow = load_allowlist(allow_path)
    unwired_keys = {m for _, m in unwired}

    if "--list" in args:
        for kind, m in wired:
            print(f"  wired    {kind:4} {m}")
        for kind, m in unwired:
            tag = "ALLOWED" if m in allow else "UNWIRED"
            print(f"  {tag:8} {kind:4} {m}")

    fail = 0
    new_unwired = sorted(m for _, m in unwired if m not in allow)
    if new_unwired:
        fail = 1
        print()
        print("FAIL: these modules have tests that NO workflow runs, and are not")
        print("      on the shrink-only allowlist:")
        for m in new_unwired:
            print(f"        {m}")
        print()
        print("      A test that runs nowhere is worse than no test — it looks like")
        print("      coverage and provides none. Wire the module into a")
        print("      `go test` / `cargo test` / `cargo nextest` step in a")
        print("      .github/workflows/ file that names its path or package.")
        print(f"      Do NOT paper over it by adding a line to {os.path.relpath(allow_path, root)}")
        print("      unless it is a genuinely-sequenced known-gap; a NEW module")
        print("      must be wired, not allowlisted.")

    stale = sorted(a for a in allow if a not in unwired_keys)
    if stale:
        fail = 1
        print()
        print(f"FAIL: these entries in {os.path.relpath(allow_path, root)} are stale —")
        print("      the module is now wired into a workflow (or no longer has tests).")
        print("      The allowlist is shrink-only: remove each line.")
        for a in stale:
            print(f"        {a}")

    if fail:
        return 1

    allowed_live = sorted(m for _, m in unwired if m in allow)
    print(
        f"OK: {len(wired)} test-bearing module(s) wired; "
        f"{len(allowed_live)} on the shrink-only allowlist (known unwired); "
        "0 new unwired."
    )
    return 0


def self_test(root):
    """Prove the guard is red-first: a planted test-bearing module named by no
    workflow must FAIL, and removing it must pass."""
    import tempfile
    import shutil

    tmp = tempfile.mkdtemp(prefix="tcw-selftest-")
    try:
        # minimal fake repo: one wired go module, one unwired go module
        os.makedirs(os.path.join(tmp, ".github", "workflows"))
        os.makedirs(os.path.join(tmp, "ops", "wired-mod"))
        os.makedirs(os.path.join(tmp, "ops", "orphan-mod"))
        for m in ("wired-mod", "orphan-mod"):
            with open(os.path.join(tmp, "ops", m, "go.mod"), "w") as fh:
                fh.write(f"module example.com/{m}\n\ngo 1.26\n")
            with open(os.path.join(tmp, "ops", m, "x_test.go"), "w") as fh:
                fh.write("package x\nimport \"testing\"\nfunc TestX(t *testing.T){}\n")
        with open(
            os.path.join(tmp, ".github", "workflows", "wired.yml"), "w"
        ) as fh:
            fh.write(
                "name: wired\non: [push]\njobs:\n  t:\n    runs-on: ubuntu-latest\n"
                "    steps:\n      - name: test\n        working-directory: ops/wired-mod\n"
                "        run: go test ./...\n"
            )

        wired, unwired, _ = compute(tmp)
        wired_names = {m for _, m in wired}
        unwired_names = {m for _, m in unwired}
        ok = True
        if "ops/wired-mod" not in wired_names:
            print("  self-test FAIL: wired module not detected as wired")
            ok = False
        if "ops/orphan-mod" not in unwired_names:
            print("  self-test FAIL: orphan module not detected as unwired")
            ok = False
        # with empty allowlist the orphan must make the gate fail
        if not unwired_names:
            print("  self-test FAIL: expected at least one unwired module")
            ok = False
        # An every-module loop wires every Go module except the ones it skips.
        os.makedirs(os.path.join(tmp, "ops", "skipped-mod"))
        with open(os.path.join(tmp, "ops", "skipped-mod", "go.mod"), "w") as fh:
            fh.write("module example.com/skipped-mod\n\ngo 1.26\n")
        with open(os.path.join(tmp, "ops", "skipped-mod", "x_test.go"), "w") as fh:
            fh.write("package x\nimport \"testing\"\nfunc TestX(t *testing.T){}\n")
        with open(os.path.join(tmp, ".github", "workflows", "all.yml"), "w") as fh:
            fh.write(
                "name: all\non: [push]\njobs:\n  t:\n    runs-on: ubuntu-latest\n"
                "    steps:\n      - name: test\n        run: |\n"
                "          for gm in $(git ls-files '*go.mod' | sed 's|/go.mod||'); do\n"
                "            [ \"$gm\" = ops/skipped-mod ] && continue\n"
                "            ( cd \"$gm\" && go test ./... )\n"
                "          done\n"
            )
        wired, unwired, _ = compute(tmp)
        wired_names = {m for _, m in wired}
        unwired_names = {m for _, m in unwired}
        if "ops/orphan-mod" not in wired_names:
            print("  self-test FAIL: every-module loop did not wire ops/orphan-mod")
            ok = False
        if "ops/skipped-mod" not in unwired_names:
            print("  self-test FAIL: module skipped by the loop counted as wired")
            ok = False
        if ok:
            print("OK: self-test — wired detected wired, orphan detected UNWIRED")
            print("    (with an empty allowlist the orphan fails the gate; red-first holds)")
            return 0
        return 1
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
