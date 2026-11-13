#!/usr/bin/env python3
"""Error codes are never reused: every code of the last release is still in the catalog.

Gate row A5 (docs/design/goatbuilder-v1-gate.md). A code printed by a released binary is in
logs, run records and pasted output that outlive the release. `goat explain` and
`goatbuilder explain` must keep answering for it with the meaning it had, so a later build
may never drop the code or give it another meaning.

This gate compares ops/errors/catalog.yaml at the last release of each binary that prints
these codes (the newest `goatbuilder/v<x.y.z>` and `goat-cli-v<x.y.z>` tags reachable from
HEAD, read with `git show`, never the network) with the catalog in the tree. It fails when a
code of such a release:

  * is absent from the tree's catalog. A code that is no longer raised stays in the
    catalog with a `retired:` mark (docs/design/error-interpretation.md, "The catalog");
    it is never deleted;
  * has a different title, and the tree's entry carries no `retired:` mark. Rewording
    `what`, `causes` and `fixes` is fine: the title is the code's meaning.

A code new since the release passes. So does a retired code, whatever its title now says.

Exit 0 ok; 1 with every defect printed; 2 when the release's catalog or the tree's cannot
be read.

usage: check-error-codes-kept.py [--repo DIR] [--base REF] [--self-test]
"""
import argparse, os, re, subprocess, sys, tempfile

try:
    import yaml
except ImportError:
    print("check-error-codes-kept: PyYAML missing (pip install pyyaml)", file=sys.stderr)
    sys.exit(2)

CATALOG = "ops/errors/catalog.yaml"
# The release tags of the binaries that print these codes: <prefix><major>.<minor>.<patch>,
# with no pre-release suffix (a `-rc.N` tag is not a release).
RELEASE_PREFIXES = ("goatbuilder/v", "goat-cli-v")


def entries(text, where):
    """The catalog's entries by code, from its YAML text."""
    data = yaml.safe_load(text) or {}
    out = {}
    for e in data.get("entries") or []:
        if isinstance(e, dict) and e.get("code"):
            out[str(e["code"])] = e
    if not out:
        raise ValueError(f"{where}: no entries")
    return out


def problems(released, now, base):
    """Every way the tree's catalog drops or changes a code of the release."""
    found = []
    for code, was in released.items():
        is_ = now.get(code)
        if is_ is None:
            found.append(f"{code} ({was.get('title', '')!r} in {base}) is not in {CATALOG}; "
                         f"keep its entry, marked retired if it is no longer raised")
            continue
        retired = str(is_.get("retired") or "").strip()
        if retired:
            continue
        if str(is_.get("title", "")).strip() != str(was.get("title", "")).strip():
            found.append(f"{code}: its title was {was.get('title', '')!r} in {base} and is now "
                         f"{is_.get('title', '')!r}; a code keeps its meaning (give the new one a new code), "
                         f"or is marked retired")
    return found


def last_release(repo, prefix="goatbuilder/v"):
    """The newest `<prefix>x.y.z` tag reachable from HEAD, or None."""
    r = subprocess.run(["git", "-C", repo, "tag", "--merged", "HEAD", "--list", f"{prefix}*"],
                       capture_output=True, text=True)
    shape = re.compile(re.escape(prefix) + r"(\d+)\.(\d+)\.(\d+)")
    found = [(tuple(int(x) for x in m.groups()), t) for t in r.stdout.split()
             if (m := shape.fullmatch(t))]
    return max(found)[1] if found else None


def last_releases(repo):
    """The last release of each binary, newest first within each; the ones that exist."""
    return [t for t in (last_release(repo, p) for p in RELEASE_PREFIXES) if t]


def at(repo, ref):
    r = subprocess.run(["git", "-C", repo, "show", f"{ref}:{CATALOG}"], capture_output=True, text=True)
    if r.returncode != 0:
        raise ValueError(f"{ref}:{CATALOG} cannot be read: {r.stderr.strip()}")
    return r.stdout


def check(repo, base):
    released = entries(at(repo, base), f"{base}:{CATALOG}")
    with open(os.path.join(repo, CATALOG)) as f:
        now = entries(f.read(), CATALOG)
    return problems(released, now, base)


def entry(code, title, **more):
    return {"code": code, "tool": "goatbuilder", "title": title, **more}


def self_test():
    """The gate fails on each way of losing a code, and passes the two ways that keep one."""
    base = "goatbuilder/v0.0.0"
    released = {"GB-001": entry("GB-001", "the plan cannot be read"),
                "GB-002": entry("GB-002", "the site is not declared")}
    cases = [
        ("an absent code", {"GB-001": released["GB-001"]}, ["GB-002"]),
        ("a changed title", {"GB-001": released["GB-001"],
                             "GB-002": entry("GB-002", "the host did not answer")}, ["GB-002"]),
        ("a retired code", {"GB-001": released["GB-001"],
                            "GB-002": entry("GB-002", "retired: the site is not declared",
                                            retired="goatbuilder/v0.6.0: standup reads the site from the plan")}, []),
        ("a new code", {**released, "GB-003": entry("GB-003", "a new refusal")}, []),
        ("a reworded what", {**released, "GB-001": entry("GB-001", "the plan cannot be read",
                                                            what="said differently")}, []),
    ]
    failed = []
    for name, now, bad in cases:
        got = problems(released, now, base)
        named = sorted({c for c in bad if any(c in p for p in got)})
        if named != sorted(bad) or (not bad and got):
            failed.append(f"{name}: expected problems for {bad}, got {got}")
    # Against a real git history: a tag, then a tree that drops a code.
    with tempfile.TemporaryDirectory() as d:
        git = lambda *a: subprocess.run(["git", "-C", d, "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", *a],
                                        capture_output=True, text=True,
                                        env={**os.environ, "GIT_AUTHOR_NAME": "self-test",
                                             "GIT_AUTHOR_EMAIL": "self-test@example.org",
                                             "GIT_COMMITTER_NAME": "self-test",
                                             "GIT_COMMITTER_EMAIL": "self-test@example.org"})
        git("init", "-q")
        os.makedirs(os.path.join(d, "ops/errors"))
        dump = lambda es: yaml.safe_dump({"schema": "goat-errors/v1", "entries": list(es.values())})
        with open(os.path.join(d, CATALOG), "w") as f:
            f.write(dump(released))
        git("add", "-A")
        made = git("commit", "-q", "-m", "release")
        if made.returncode != 0:
            raise SystemExit(f"self-test: git commit failed: {made.stderr.strip()}")
        git("tag", "goatbuilder/v0.0.1")
        git("tag", "goatbuilder/v0.0.2-rc.1")
        git("tag", "goatbuilder/v0.0.10-rc.1")
        git("tag", "goat-cli-v0.9.0")
        git("tag", "goat-cli-v0.10.0")
        with open(os.path.join(d, CATALOG), "w") as f:
            f.write(dump({"GB-001": released["GB-001"]}))
        if last_release(d) != "goatbuilder/v0.0.1":
            failed.append(f"the last release is goatbuilder/v0.0.1, not {last_release(d)} (a pre-release is not one)")
        if last_releases(d) != ["goatbuilder/v0.0.1", "goat-cli-v0.10.0"]:
            failed.append(f"each binary's last release, by version not by text: {last_releases(d)}")
        got = check(d, "goatbuilder/v0.0.1")
        if not any("GB-002" in p for p in got):
            failed.append(f"a tree that drops GB-002 against its tag: {got}")
    if failed:
        print("check-error-codes-kept: SELF-TEST FAILED")
        for f in failed:
            print(f"  {f}")
        return 1
    print("check-error-codes-kept: self-test ok (an absent code and a changed title fail; "
          "a retired code, a new code and a reworded `what` pass; each binary's last release is "
          "its highest x.y.z, and a pre-release is not one)")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--repo", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
    ap.add_argument("--base", action="append",
                    help="a release to compare with, repeatable (default: the newest goatbuilder/v<x.y.z> "
                         "and goat-cli-v<x.y.z> tags reachable from HEAD)")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    repo = os.path.abspath(a.repo)
    bases = a.base or last_releases(repo)
    if not bases:
        print("check-error-codes-kept: no goatbuilder/v<x.y.z> or goat-cli-v<x.y.z> tag is reachable from "
              "HEAD (a shallow clone has none: check out with fetch-depth: 0), or pass --base <tag>",
              file=sys.stderr)
        return 2
    found = []
    try:
        for base in bases:
            found += check(repo, base)
    except (OSError, ValueError, yaml.YAMLError) as e:
        print(f"check-error-codes-kept: {e}", file=sys.stderr)
        return 2
    base = " and ".join(bases)
    if found:
        print(f"check-error-codes-kept: {len(found)} code(s) of {base} not kept:")
        for p in found:
            print(f"  {p}")
        print("A code a release printed is never removed or given another meaning. Keep its entry; if it is no "
              "longer raised, mark it `retired: \"<release tag>: <why>\"`, such as goatbuilder/v0.6.0 (docs/design/error-interpretation.md).")
        return 1
    print(f"check-error-codes-kept: ok, every code of {base} is kept")
    return 0


if __name__ == "__main__":
    sys.exit(main())
