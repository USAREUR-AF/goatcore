#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""goatbuilder ships no environment or programme name.

What a person outside this project reads or runs: the binary's non-test source
(including `///` help text), its English message catalog, the shared error
catalog and the goatbuilder reference pages. A project's own names belong in its
config (docs/design/goatbuilder-portability.md).

ops/goatbuilder/portability-baseline.txt holds the count per file that exists
today. A file above its count fails; a file below its count fails too, so the
list only shrinks.

Usage: check-goatbuilder-portable.py [--write] [--self-test]
Rollback: rm this file, the baseline and its step in goatbuilder-pr.yml.
"""
import pathlib
import re
import sys
import tempfile

# The example goatnet's name (zagros) is the tutorials' vocabulary, not a
# project's own name, so it is not a needle here.
NEEDLES = re.compile(r"nova|usareur|goat-maintainer|nethost|\bthor\b", re.I)
BASELINE = "ops/goatbuilder/portability-baseline.txt"


def shipped_lines(path: pathlib.Path):
    """The lines of a file that reach a reader. A Rust file ends at its tests."""
    for line in path.read_text(encoding="utf-8").splitlines():
        if path.suffix == ".rs":
            if line.strip() == "#[cfg(test)]":
                return
            stripped = line.lstrip()
            if stripped.startswith("//") and not stripped.startswith("///"):
                continue
        elif path.suffix == ".yaml" and line.lstrip().startswith("#"):
            continue
        yield line


def scanned(root: pathlib.Path):
    yield from sorted(root.glob("crates/goatbuilder/src/*.rs"))
    yield root / "crates/goatbuilder/i18n/en/goatbuilder.ftl"
    yield root / "ops/errors/catalog.yaml"
    yield from sorted(root.glob("docs/reference/goatbuilder/*.md"))


def counts(root: pathlib.Path) -> dict:
    out = {}
    for path in scanned(root):
        if not path.is_file():
            continue
        n = sum(1 for line in shipped_lines(path) if NEEDLES.search(line))
        if n:
            out[path.relative_to(root).as_posix()] = n
    return out


def read_baseline(root: pathlib.Path) -> dict:
    out = {}
    path = root / BASELINE
    if path.is_file():
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.strip() and not line.startswith("#"):
                name, n = line.rsplit(None, 1)
                out[name] = int(n)
    return out


def write_baseline(root: pathlib.Path) -> None:
    head = "# Lines naming an environment or programme, per file (ops/ci/check-goatbuilder-portable.py).\n# Only shrinks. Regenerate after removing names: python3 ops/ci/check-goatbuilder-portable.py --write\n"
    rows = "".join(f"{name} {n}\n" for name, n in sorted(counts(root).items()))
    (root / BASELINE).write_text(head + rows, encoding="utf-8")


def problems(root: pathlib.Path) -> list:
    now, base = counts(root), read_baseline(root)
    errs = []
    for name in sorted(set(now) | set(base)):
        have, allowed = now.get(name, 0), base.get(name, 0)
        if have > allowed:
            found = [f"    {line.strip()[:110]}" for line in shipped_lines(root / name) if NEEDLES.search(line)]
            errs.append(f"{name}: {have} lines name an environment or programme, {allowed} allowed. Read the value from the config instead:\n" + "\n".join(found[:12]))
        elif have < allowed:
            errs.append(f"{name}: {have} lines now, the baseline says {allowed}. Lower it: python3 ops/ci/check-goatbuilder-portable.py --write")
    return errs


def self_test() -> int:
    with tempfile.TemporaryDirectory() as d:
        root = pathlib.Path(d)
        src = root / "crates/goatbuilder/src"
        src.mkdir(parents=True)
        (root / "ops/goatbuilder").mkdir(parents=True)
        f = src / "a.rs"
        f.write_text('/// e.g. nethost-dev\n// nethost in a comment\nfn a() {}\n#[cfg(test)]\nmod t { const X: &str = "nethost"; }\n')
        cases = []
        cases.append(("help text counts, comments and tests do not", counts(root) == {"crates/goatbuilder/src/a.rs": 1}))
        cases.append(("a new name with no baseline fails", len(problems(root)) == 1))
        write_baseline(root)
        cases.append(("the written baseline passes", problems(root) == []))
        f.write_text('/// e.g. nethost-dev\n/// or usareur-af\nfn a() {}\n')
        cases.append(("a second name fails", len(problems(root)) == 1))
        f.write_text("/// e.g. <site>\nfn a() {}\n")
        cases.append(("a name removed without lowering the baseline fails", len(problems(root)) == 1))
    for label, good in cases:
        print(f"{'ok' if good else 'FAIL'}: {label}")
    return 0 if all(good for _, good in cases) else 1


def main(argv: list) -> int:
    root = pathlib.Path(__file__).resolve().parents[2]
    if "--self-test" in argv:
        return self_test()
    if "--write" in argv:
        write_baseline(root)
        print(f"wrote {BASELINE}")
        return 0
    errs = problems(root)
    for e in errs:
        print(f"FAIL: {e}")
    if errs:
        return 1
    print(f"ok: {sum(counts(root).values())} lines in the baseline, none new")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
