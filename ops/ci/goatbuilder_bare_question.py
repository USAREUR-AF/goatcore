#!/usr/bin/env python3
"""goatbuilder_bare_question.py — no `?` in goatbuilder passes up an error without its code.

Gate row C5 (docs/design/goatbuilder-v1-gate.md): every error goatbuilder raises
carries its catalog code. `crates/goatbuilder/tests/errors_coded_in_source.rs`
reads the places an error is made with an opener (`anyhow!`, `bail!`,
`.context`). It cannot see a bare `?` that passes up a standard library, serde
or other foreign error, because there is no opener there. Such an error leaves
`main.rs` as `goatbuilder: <text>`, with no code.

This finds those places with the compiler, so a `?` whose expression spans
several lines is found too. In a copy of the tree, goatbuilder's `anyhow::Result`
is replaced by an error type that converts only from `anyhow::Error` and
`goat_errors::Coded`. `cargo check` of that copy then fails at every `?` that
passes up any other error, with "`?` couldn't convert the error", and at every
`.into()` of such an error into the function's error, as an unsatisfied `From`
bound. The copy has many other type errors; only those are read. A canary (a bare `?` the copy
adds) proves the compiler reached them. The tree itself is never changed.

Each place is keyed by its file and the trimmed text of its line, so an edit
elsewhere in a file does not move it. The places on today's tree are listed in
crates/goatbuilder/tests/bare-question-baseline.txt, which only shrinks: a place
not listed fails the gate, and a listed place that is gone fails it too, so its
line is removed in the change that codes it.

usage: goatbuilder_bare_question.py [--write] | --self-test
  CARGO      the cargo command (default: cargo), e.g. "cargo +1.100.0"
  CARGO_TARGET_DIR  where the copy is built (default: a directory under the copy)
exit 0 every place listed and none gone · 1 a new place, or a listed place gone · 2 cannot run
"""
import collections
import json
import os
import re
import shlex
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATE = "crates/goatbuilder"
BASELINE = ROOT / CRATE / "tests/bare-question-baseline.txt"
SCAN_MODULE = "scanerr"
CANARY = "goatbuilder-bare-question-canary"

SCANERR_RS = f"""//! Added by ops/ci/goatbuilder_bare_question.py to a copy of the tree only.
#[derive(Debug)]
pub struct E(pub anyhow::Error);
impl From<anyhow::Error> for E {{ fn from(e: anyhow::Error) -> Self {{ E(e) }} }}
impl From<goat_errors::Coded> for E {{ fn from(e: goat_errors::Coded) -> Self {{ E(anyhow::Error::from(e)) }} }}
impl std::fmt::Display for E {{ fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ std::fmt::Display::fmt(&self.0, f) }} }}
impl E {{
    pub fn downcast_ref<T: std::fmt::Display + std::fmt::Debug + Send + Sync + 'static>(&self) -> Option<&T> {{ self.0.downcast_ref::<T>() }}
    pub fn chain(&self) -> anyhow::Chain<'_> {{ self.0.chain() }}
    pub fn root_cause(&self) -> &(dyn std::error::Error + 'static) {{ self.0.root_cause() }}
}}
pub type Result<T, X = E> = std::result::Result<T, X>;
#[allow(dead_code)]
fn canary() -> Result<()> {{
    std::fs::read("{CANARY}")?;
    Ok(())
}}
"""

USE_LIST = re.compile(r"use anyhow::\{([^}]*)\};")
# A `?` that cannot convert the error, and an `.into()` (or `From::from`) of a
# foreign error into the function's error, which the compiler names as an
# unsatisfied `From` bound on the scan's type.
PASSES_UP = re.compile(r"`\?` couldn't convert the error|the trait bound `E: (std::convert::)?From<")


def use_without_result(m):
    """`use anyhow::{Context, Result};` as `use anyhow::{Context}; use crate::scanerr::Result;`, on one line."""
    items = [i.strip() for i in m.group(1).split(",") if i.strip()]
    if "Result" not in items:
        return m.group(0)
    rest = [i for i in items if i != "Result"]
    head = f"use anyhow::{{{', '.join(rest)}}}; " if rest else ""
    return f"{head}use crate::{SCAN_MODULE}::Result;"


def transform(src: Path) -> None:
    """Rewrite the copy's goatbuilder sources in place to use the scan's error type."""
    main = src / "main.rs"
    text = main.read_text()
    m = re.search(r"^mod \w+;", text, re.M)
    if not m:
        raise RuntimeError(f"{main}: no `mod` line to put the scan module beside")
    main.write_text(text[: m.start()] + f"mod {SCAN_MODULE};\n" + text[m.start():])
    (src / f"{SCAN_MODULE}.rs").write_text(SCANERR_RS)
    for p in src.rglob("*.rs"):
        if p.name == f"{SCAN_MODULE}.rs":
            continue
        t = p.read_text()
        n = USE_LIST.sub(use_without_result, t).replace("anyhow::Result", f"crate::{SCAN_MODULE}::Result")
        if n != t:
            p.write_text(n)


def places(messages, crate_src_prefix):
    """The places the compiler names: (file under the crate, trimmed line text), one per `?`."""
    out = []
    for line in messages:
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("reason") != "compiler-message":
            continue
        d = m.get("message") or {}
        if d.get("level") != "error" or not PASSES_UP.search(d.get("message", "")):
            continue
        spans = [s for s in d.get("spans", []) if s.get("is_primary")]
        if not spans:
            continue
        s = spans[0]
        name = s.get("file_name", "")
        if not name.startswith(crate_src_prefix):
            continue
        rel = name[len(crate_src_prefix):]
        text = (s.get("text") or [{}])[0].get("text", "").strip()
        out.append((rel, text))
    return out


def key(place):
    return f"{place[0]} :: {place[1]}"


def compare(found, listed):
    """(new places, listed places gone), each a list of keys, counted as multisets."""
    f = collections.Counter(key(p) for p in found)
    b = collections.Counter(listed)
    new = sorted((f - b).elements())
    gone = sorted((b - f).elements())
    return new, gone


def read_baseline(path):
    if not path.exists():
        return []
    return [l.rstrip("\n") for l in path.read_text().splitlines() if l.strip() and not l.startswith("#")]


HEADER = """# Places in crates/goatbuilder/src/ where a `?` passes up an error with no
# catalog code (ops/ci/goatbuilder_bare_question.py). This list only shrinks:
# the change that codes a place removes its line. A new place is coded, not
# listed. One line per `?`: <file> :: <the trimmed text of its line>.
"""


def scan(root=ROOT):
    """Copy the tree, transform it, run cargo check, and return the places found."""
    files = subprocess.run(["git", "-C", str(root), "ls-files", "-z"], check=True, capture_output=True).stdout.split(b"\0")
    with tempfile.TemporaryDirectory(prefix="gb-bare-question-") as tmp:
        dst = Path(tmp) / "tree"
        for f in files:
            if not f:
                continue
            rel = f.decode()
            s = root / rel
            d = dst / rel
            d.parent.mkdir(parents=True, exist_ok=True)
            if s.is_symlink():
                os.symlink(os.readlink(s), d)
                continue
            if not s.is_file():
                continue
            d.write_bytes(s.read_bytes())
            os.chmod(d, s.stat().st_mode & 0o777)
        transform(dst / CRATE / "src")
        cargo = shlex.split(os.environ.get("CARGO", "cargo"))
        env = dict(os.environ)
        env.setdefault("CARGO_TARGET_DIR", str(Path(tmp) / "target"))
        r = subprocess.run(
            cargo + ["check", "--locked", "-p", "goatbuilder", "--bin", "goatbuilder", "--message-format=json"],
            cwd=dst / "crates", env=env, capture_output=True, text=True,
        )
        found = places(r.stdout.splitlines(), "goatbuilder/src/")
        canary = [p for p in found if p[0] == f"{SCAN_MODULE}.rs"]
        if not canary:
            tail = "\n".join(r.stderr.splitlines()[-20:])
            raise RuntimeError(f"the scan's canary was not found, so the compiler did not reach the type checks:\n{tail}")
        return [p for p in found if p[0] != f"{SCAN_MODULE}.rs"]


def self_test():
    import unittest
    here = Path(__file__).resolve().parent
    suite = unittest.defaultTestLoader.discover(str(here), pattern="test_goatbuilder_bare_question.py")
    return 0 if unittest.TextTestRunner(verbosity=1).run(suite).wasSuccessful() else 1


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    if argv not in ([], ["--write"]):
        print("usage: goatbuilder_bare_question.py [--write] | --self-test", file=sys.stderr)
        return 2
    try:
        found = scan()
    except (OSError, RuntimeError, subprocess.CalledProcessError) as e:
        print(f"goatbuilder-bare-question: cannot run: {e}", file=sys.stderr)
        return 2
    if argv == ["--write"]:
        BASELINE.write_text(HEADER + "".join(key(p) + "\n" for p in sorted(found)))
        print(f"goatbuilder-bare-question: wrote {BASELINE.relative_to(ROOT)} ({len(found)} place(s))")
        return 0
    new, gone = compare(found, read_baseline(BASELINE))
    for k in new:
        print(f"new: {k}\n    code it: .map_err(|e| goat_errors::coded(\"GB-nnn\", ...)) before the `?`")
    for k in gone:
        print(f"no longer found: {k}\n    remove its line from {BASELINE.relative_to(ROOT)}")
    if new or gone:
        print(f"goatbuilder-bare-question: {len(new)} new place(s), {len(gone)} listed place(s) gone")
        return 1
    print(f"goatbuilder-bare-question: ok, {len(found)} listed place(s), none new")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
