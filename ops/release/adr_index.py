#!/usr/bin/env python3
"""adr_index: the ADR index's rows for new ADRs, rendered from the ADR files (ADR 1231).

docs/adr/INDEX.md lists every ADR. The rows of ADR 1234 and below are kept by
hand, as they always were. Every ADR numbered above 1234 is listed in one table
rendered from the ADR files themselves, between two marker lines, so a pull
request that adds an ADR edits no shared file.

  adr_index.py check        exit 0 when every ADR above 1234 is readable and the
                            rendered table equals its render; exit 1 with each
                            reason when not
  adr_index.py write        rewrite the rendered table from the ADR files
  adr_index.py --self-test  prove each refusal on a copy, then run the check on
                            this repository

  --repo DIR   the repository root (default: the one this script is in)

What each ADR above 1234 must declare, as docs/adr/TEMPLATE.md writes it:

  * its file name, `NNNN-<slug>.md`, one file per number;
  * its first line, `# ADR NNNN — <title>`, with the file's number;
  * its header table, before the first `## ` heading, with a `| Status | ... |`
    row. A Status written another way (a `**Status:**` line, or YAML front
    matter) is refused with the reason, never read by guessing;
  * optionally `| Amends | ... |` and `| Supersedes | ... |` rows; each ADR
    number they name goes into the row's Notes.

A row is `| NNNN | [<title>](<file>) | <Status, verbatim> | <Notes> |`, in
number order. Notes is `Amends NNNN; supersedes NNNN`, or `—` when there is
neither. The table is left out entirely while no ADR is above 1234.

Python's standard library only.
"""
from __future__ import annotations

import argparse
import re
import shutil
import sys
import tempfile
from pathlib import Path

ADR_DIR = "docs/adr"
INDEX = "docs/adr/INDEX.md"
# The last ADR whose INDEX row is kept by hand. Rows of ADRs above it are rendered.
LAST_HAND_KEPT = 1234
BEGIN = (f"<!-- BEGIN generated: adr-index, ADRs above {LAST_HAND_KEPT} from their own files; "
         "run python3 ops/release/adr_index.py write -->")
END = "<!-- END generated: adr-index -->"
HEADER = "| # | Title | Status | Notes |\n|---|---|---|---|"
FILE = re.compile(r"^(\d{4})-[a-z0-9][a-z0-9-]*\.md$")
H1 = re.compile(r"^# ADR (\d{4}) — (.+)$")
ROW = re.compile(r"^\|\s*(\d{4})\s*\|")


class Refused(Exception):
    """One or more reasons, each a line."""


def field(header: list[str], name: str) -> str | None:
    for line in header:
        m = re.match(r"^\|\s*" + re.escape(name) + r"\s*\|(.*)\|\s*$", line)
        if m:
            return m.group(1).strip()
    return None


def numbers(value: str | None) -> list[str]:
    """The ADR numbers a header cell names; `—` or empty names none."""
    if not value or value.strip() in ("—", "-", "–"):
        return []
    seen = []
    for n in re.findall(r"(?<![\d])(\d{4})(?![\d])", value):
        if n not in seen:
            seen.append(n)
    return seen


def read_adr(path: Path) -> dict:
    """One ADR's row, or Refused naming what the file must say instead."""
    name = path.name
    num = name[:4]
    lines = path.read_text(encoding="utf-8").split("\n")
    errors = []
    if lines and lines[0].strip() == "---":
        raise Refused([f"{name}: the header is YAML front matter; write it as the "
                       f"header table of {ADR_DIR}/TEMPLATE.md, with a `| Status | ... |` row"])
    m = H1.match(lines[0]) if lines else None
    if not m:
        errors.append(f"{name}: the first line is not `# ADR {num} — <title>`")
    elif m.group(1) != num:
        errors.append(f"{name}: the first line names ADR {m.group(1)}, the file name {num}")
    end = next((i for i, l in enumerate(lines) if l.startswith("## ")), len(lines))
    header = lines[1:end]
    status = field(header, "Status")
    if status is None:
        other = next((l.strip() for l in header
                      if re.match(r"^\s*(?:[-*]\s*)?\*\*Status:?\*\*", l) or re.match(r"^status:", l, re.I)), None)
        if other:
            errors.append(f"{name}: the Status is written as {other!r}; write it as the "
                          f"`| Status | ... |` row of the header table ({ADR_DIR}/TEMPLATE.md)")
        else:
            errors.append(f"{name}: no `| Status | ... |` row in the header table before the first `## ` heading")
    elif not status:
        errors.append(f"{name}: the Status row is empty")
    if errors:
        raise Refused(errors)
    notes = []
    amends = numbers(field(header, "Amends"))
    supersedes = numbers(field(header, "Supersedes"))
    if amends:
        notes.append("Amends " + ", ".join(amends))
    if supersedes:
        notes.append(("supersedes " if notes else "Supersedes ") + ", ".join(supersedes))
    return {"num": num, "file": name, "title": m.group(2).strip(), "status": status,
            "notes": "; ".join(notes) or "—"}


def read_adrs(repo: Path) -> list[dict]:
    """Every ADR above LAST_HAND_KEPT, in number order, or Refused."""
    by_num: dict[str, list[Path]] = {}
    for path in sorted((repo / ADR_DIR).glob("[0-9][0-9][0-9][0-9]-*.md")):
        if int(path.name[:4]) > LAST_HAND_KEPT:
            by_num.setdefault(path.name[:4], []).append(path)
    adrs, errors = [], []
    for num in sorted(by_num):
        paths = by_num[num]
        if len(paths) > 1:
            errors.append(f"ADR {num} has {len(paths)} files: " + ", ".join(p.name for p in paths))
            continue
        if not FILE.match(paths[0].name):
            errors.append(f"{paths[0].name}: the file name is not NNNN-<slug>.md in lower case letters, digits and hyphens")
            continue
        try:
            adrs.append(read_adr(paths[0]))
        except Refused as r:
            errors.extend(r.args[0])
    if errors:
        raise Refused(errors)
    return adrs


def cell(text: str) -> str:
    return " ".join(text.split()).replace("|", "\\|")


def rendered(adrs: list[dict]) -> list[str]:
    if not adrs:
        return [BEGIN, END]
    rows = [f"| {a['num']} | [{cell(a['title'])}]({a['file']}) | {cell(a['status'])} | {cell(a['notes'])} |"
            for a in adrs]
    return [BEGIN, ""] + HEADER.split("\n") + rows + ["", END]


def region(lines: list[str]) -> tuple[int, int]:
    begins = [i for i, l in enumerate(lines) if l == BEGIN]
    ends = [i for i, l in enumerate(lines) if l == END]
    if len(begins) != 1 or len(ends) != 1 or begins[0] > ends[0]:
        raise Refused([f"{INDEX}: expected one BEGIN marker line followed by one END marker line, "
                       f"found {len(begins)} and {len(ends)}"])
    b, e = begins[0], ends[0]
    hand = [l for l in lines[:b] + lines[e + 1:] if (m := ROW.match(l)) and int(m.group(1)) > LAST_HAND_KEPT]
    if hand:
        raise Refused([f"{INDEX}: a row written by hand for ADR {ROW.match(l).group(1)}; rows above "
                       f"{LAST_HAND_KEPT} are rendered: delete it and run `python3 ops/release/adr_index.py write`"
                       for l in hand])
    return b, e


def check(repo: Path) -> list[str]:
    try:
        adrs = read_adrs(repo)
        lines = (repo / INDEX).read_text(encoding="utf-8").split("\n")
        b, e = region(lines)
    except Refused as r:
        return list(r.args[0])
    if lines[b:e + 1] != rendered(adrs):
        return [f"{INDEX}: the rendered table differs from the ADR files; run "
                "`python3 ops/release/adr_index.py write` (a hand edit there is lost: edit the ADR's header instead)"]
    return []


def write(repo: Path) -> None:
    adrs = read_adrs(repo)
    path = repo / INDEX
    lines = path.read_text(encoding="utf-8").split("\n")
    b, e = region(lines)
    lines[b:e + 1] = rendered(adrs)
    path.write_text("\n".join(lines), encoding="utf-8")


# --- self-test ---------------------------------------------------------------

ADR_TEXT = """# ADR {num} — {title}

| Field | Value |
|---|---|
{status}| Date | 2099-01-01 |
| Repo scope | goat core |
{extra}| Supersedes | {supersedes} |
| Superseded by | — |

## TL;DR (for a layperson)

A test decision.
"""


def _adr(repo: Path, num: int, title: str = "A test decision", status: str = "| Status | Proposed |\n",
         extra: str = "", supersedes: str = "—", slug: str = "a-test-decision") -> Path:
    p = repo / ADR_DIR / f"{num}-{slug}.md"
    p.write_text(ADR_TEXT.format(num=num, title=title, status=status, extra=extra, supersedes=supersedes),
                 encoding="utf-8")
    return p


def self_test(repo: Path) -> int:
    failures = []

    def expect(what: str, ok: bool) -> None:
        print(f"  {'ok  ' if ok else 'FAIL'} {what}")
        if not ok:
            failures.append(what)

    print("adr_index.py self-test, on a copy of docs/adr:")
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp) / "base"
        shutil.copytree(repo / ADR_DIR, base / ADR_DIR, ignore=shutil.ignore_patterns("diagrams"))

        def fresh(name: str) -> Path:
            d = Path(tmp) / name
            shutil.copytree(base, d)
            return d

        expect("the copy passes the check", check(base) == [])
        if not read_adrs(base):
            expect("with no ADR above the last hand-kept one, the region is its two marker lines only",
                   f"{BEGIN}\n{END}" in (base / INDEX).read_text(encoding="utf-8"))

        r = fresh("new")
        before = (r / INDEX).read_text(encoding="utf-8")
        _adr(r, 9002, title="The second test decision", slug="second")
        _adr(r, 9001, extra="| Amends | [ADR 1225](1225-x.md) decision 3 |\n", supersedes="ADR-1100, ADR-1101")
        expect("a new ADR, before `write`, is refused", any("differs" in m for m in check(r)))
        write(r)
        after = (r / INDEX).read_text(encoding="utf-8")
        expect("after `write`, the check passes", check(r) == [])
        expect("the rows are rendered in number order, with the title linked to the file",
               "| 9001 | [A test decision](9001-a-test-decision.md) | Proposed |" in after
               and after.index("| 9001 |") < after.index("| 9002 |"))
        expect("Notes carries the Amends and Supersedes numbers",
               "| Proposed | Amends 1225; supersedes 1100, 1101 |" in after)
        expect("an ADR that amends and supersedes nothing has `—` as its Notes",
               "| [The second test decision](9002-second.md) | Proposed | — |" in after)
        b = before.split(BEGIN)
        a = after.split(BEGIN)
        expect("every line outside the region is unchanged, byte for byte",
               b[0] == a[0] and before.split(END)[1] == after.split(END)[1])

        r = fresh("hand-edit")
        _adr(r, 9001)
        write(r)
        p = r / INDEX
        row = "(9001-a-test-decision.md) | Proposed |"
        text = p.read_text(encoding="utf-8")
        p.write_text(text.replace(row, row.replace("Proposed", "Accepted"), 1), encoding="utf-8")
        expect("a status changed by hand in the rendered table is refused",
               any("differs" in m for m in check(r)))

        r = fresh("bullet")
        _adr(r, 9001, status="")
        q = r / ADR_DIR / "9001-a-test-decision.md"
        q.write_text(q.read_text(encoding="utf-8").replace("| Field | Value |", "- **Status:** Proposed\n\n| Field | Value |"),
                     encoding="utf-8")
        msgs = check(r)
        expect("an ADR above the last hand-kept one with a bullet Status is refused, naming the form, not read",
               any("9001-a-test-decision.md: the Status is written as '- **Status:** Proposed'" in m for m in msgs))

        r = fresh("yaml")
        (r / ADR_DIR / "9001-yaml.md").write_text("---\nstatus: Accepted\ntitle: A decision\n---\n\n# A decision\n",
                                                  encoding="utf-8")
        expect("an ADR above the last hand-kept one with YAML front matter is refused, naming the form",
               any("9001-yaml.md: the header is YAML front matter" in m for m in check(r)))

        r = fresh("no-status")
        _adr(r, 9001, status="")
        expect("an ADR with no Status at all is refused",
               any("no `| Status | ... |` row" in m for m in check(r)))

        r = fresh("h1")
        _adr(r, 9001, title="x").write_text(
            (r / ADR_DIR / "9001-a-test-decision.md").read_text(encoding="utf-8").replace("# ADR 9001", "# ADR 9009"),
            encoding="utf-8")
        expect("a first line naming another number is refused",
               any("names ADR 9009, the file name 9001" in m for m in check(r)))

        r = fresh("two-files")
        _adr(r, 9001)
        _adr(r, 9001, slug="another")
        expect("two files with one number are refused", any("ADR 9001 has 2 files" in m for m in check(r)))

        r = fresh("hand-row")
        p = r / INDEX
        p.write_text(p.read_text(encoding="utf-8").replace(BEGIN, "| 9003 | [x](9003-x.md) | Proposed | |\n\n" + BEGIN, 1),
                     encoding="utf-8")
        expect("a row written by hand for an ADR above the last hand-kept one is refused",
               any("a row written by hand for ADR 9003" in m for m in check(r)))

        r = fresh("old")
        q = sorted((r / ADR_DIR).glob("1234-*.md"))
        if q:
            q[0].write_text(q[0].read_text(encoding="utf-8").replace("| Status |", "| Stat |", 1), encoding="utf-8")
        expect("an ADR at or below the last hand-kept one is not read (its row stays by hand)", check(r) == [])

    print("the check, on this repository:")
    problems = check(repo)
    for m in problems:
        print(f"  {m}")
    expect("this repository's index equals its render", problems == [])
    if failures:
        print(f"self-test FAILED: {len(failures)} case(s)")
        return 1
    print("self-test passed")
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2])
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("command", nargs="?", choices=("check", "write"))
    a = ap.parse_args(argv)
    if a.self_test:
        return self_test(a.repo)
    try:
        if a.command == "write":
            write(a.repo)
            print(f"adr_index: wrote the rendered table of {INDEX}")
            return 0
        problems = check(a.repo)
        for m in problems:
            print(f"adr_index: {m}", file=sys.stderr)
        if not problems:
            print(f"adr_index: ok, the rows of ADRs above {LAST_HAND_KEPT} equal their render")
        return 1 if problems else 0
    except Refused as r:
        for m in r.args[0]:
            print(f"adr_index: {m}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
