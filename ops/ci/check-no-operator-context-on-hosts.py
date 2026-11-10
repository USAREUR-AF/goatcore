#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""No acceptance row needs an operator context on a management host.

The live acceptance rows run `goat` on a goatnet's management host (goat_via,
through its door for a goatnet the laptop is not meshed to). They run there as
non-person identities only (ADR 1207): the intake rows as the sponsor identity
`acceptance` (goat_sponsor_context), the fabric rows as `assay-probe`
(probe_context). Operator keys never go on a host.

Fails when:
  1. an entry in ops/validation/goatnet-runners.yaml names any context: the
     runners file says where the rows run, the values file says as whom;
  2. an ops/validation/values-*.yaml names a sponsor or probe context other
     than the non-person ones;
  3. a design doc, runbook, reference page, script or goatbuilder source names
     the retired operator-context field (`goat_op_context`, `--op-context`) or
     says a host must hold `goat_context`.

Usage: check-no-operator-context-on-hosts.py [--self-test]
"""
from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

NON_PERSON = {"goat_sponsor_context": "acceptance", "probe_context": "assay-probe"}
SCANNED = ["docs/design/*.md", "docs/operations/*.md", "docs/reference/**/*.md",
           "ops/scripts/*", "ops/validation/*", "ops/e2e/**/*.py", "ops/e2e/**/*.sh",
           "crates/goatbuilder/src/**/*.rs"]
RETIRED = [
    (re.compile(r"goat_op_context"), "names the retired operator context field goat_op_context"),
    (re.compile(r"--op-context"), "names the retired --op-context flag"),
    (re.compile(r"must hold[^.]{0,120}`?goat_context\b"), "says a host must hold goat_context (an operator context)"),
]
SELF = "ops/ci/check-no-operator-context-on-hosts.py"


def _scalar(text: str, key: str) -> str | None:
    m = re.search(rf"(?m)^{re.escape(key)}:\s*(\S[^#\n]*)", text)
    return m.group(1).strip().strip("\"'") if m else None


def check(repo: Path) -> list[str]:
    errs: list[str] = []
    runners = repo / "ops/validation/goatnet-runners.yaml"
    if runners.is_file():
        for n, line in enumerate(runners.read_text().splitlines(), 1):
            m = re.match(r"^\s{4}([a-z_]*context[a-z_]*):", line)
            if m:
                errs.append(f"{runners.relative_to(repo)}:{n}: runner names a context ({m.group(1)}); "
                            "the rows run as the values file's non-person contexts (ADR 1207)")
    for values in sorted((repo / "ops/validation").glob("values-*.yaml")):
        text = values.read_text()
        for key, want in NON_PERSON.items():
            got = _scalar(text, key)
            if got is not None and got != want:
                errs.append(f"{values.relative_to(repo)}: {key} is {got!r}, want the non-person {want!r} (ADR 1207)")
    seen: set[Path] = set()
    for pattern in SCANNED:
        for path in sorted(repo.glob(pattern)):
            if not path.is_file() or path in seen or path.relative_to(repo).as_posix() == SELF:
                continue
            seen.add(path)
            try:
                text = path.read_text()
            except UnicodeDecodeError:
                continue
            for n, line in enumerate(text.splitlines(), 1):
                for rx, why in RETIRED:
                    if rx.search(line):
                        errs.append(f"{path.relative_to(repo)}:{n}: {why}; operator keys never go on a host (ADR 1207)")
    return errs


def _fixture(root: Path, *, runner_ctx: bool, values_ctx: str, doc_line: str) -> None:
    val = root / "ops/validation"
    val.mkdir(parents=True)
    extra = "    goat_op_context: x-op\n" if runner_ctx else ""
    (val / "goatnet-runners.yaml").write_text(
        f"schema: goatnet-runners/v0\ngoatnets:\n  x:\n    name: x\n    goat_via: \"door:x\"\n{extra}"
        "    values: ops/validation/values-x.yaml\n")
    (val / "values-x.yaml").write_text(f'goat_context: "x"\ngoat_sponsor_context: "{values_ctx}"\n')
    docs = root / "docs/design"
    docs.mkdir(parents=True)
    (docs / "x.md").write_text(f"# x\n\n{doc_line}\n")


def self_test() -> int:
    clean = "The host holds `acceptance` for the intake rows."
    cases = [
        ("clean", dict(runner_ctx=False, values_ctx="acceptance", doc_line=clean), 0),
        ("runner names an operator context", dict(runner_ctx=True, values_ctx="acceptance", doc_line=clean), 1),
        ("values sponsor is an operator context", dict(runner_ctx=False, values_ctx="x-op", doc_line=clean), 1),
        ("doc says the host must hold goat_context",
         dict(runner_ctx=False, values_ctx="acceptance", doc_line="The host must hold `goat_context` as named."), 1),
        ("doc names --op-context", dict(runner_ctx=False, values_ctx="acceptance", doc_line="Pass --op-context x-op."), 1),
    ]
    bad = 0
    for name, kw, want in cases:
        with tempfile.TemporaryDirectory() as d:
            _fixture(Path(d), **kw)
            got = len(check(Path(d)))
            ok = (got == 0) == (want == 0)
            print(f"  {'ok  ' if ok else 'FAIL'} {name}: {got} error(s)")
            bad += not ok
    print("self-test: " + ("PASS" if not bad else f"{bad} case(s) wrong"))
    return 1 if bad else 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    repo = Path(__file__).resolve().parents[2]
    errs = check(repo)
    for e in errs:
        print(f"FAIL: {e}")
    if errs:
        return 1
    print("ok: no acceptance row needs an operator context on a host")
    return 0


if __name__ == "__main__":
    sys.exit(main())
