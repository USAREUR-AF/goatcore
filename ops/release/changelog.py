#!/usr/bin/env python3
"""changelog: goatbuilder's changelog, rendered from one file per entry (ADR 1231).

An entry is a file in crates/goatbuilder/changelog.d/, so a pull request that
adds one edits no shared file. CHANGELOG.md's "Unreleased" section is rendered
from those files, between two marker lines. A release folds them into a dated
section and removes them. A released section is never rendered again: after
the release it is text like any other, and may be edited by hand.

  changelog.py check                     exit 0 when every entry is well formed and the
                                         Unreleased section equals its render; exit 1 with
                                         each reason when not
  changelog.py write                     rewrite the Unreleased section from the entries
  changelog.py release <version> <date>  fold the entries into "## <version> — <date>",
                                         remove them, and empty the Unreleased section
  changelog.py --self-test               prove the check and the release on a copy, then
                                         run the check on this repository

  --repo DIR   the repository root (default: the one this script is in)

An entry file is named <slug>.md (lower case letters, digits and hyphens), and
starts with a header between two `---` lines:

    ---
    heading: For someone deploying
    title: Fixed: a federate run killed in phase 11 resumes
    ---

    The body, in Markdown, with no heading of level 1, 2 or 3.

`heading` is one of the three fixed headings below. `title` is the entry's
one-line heading. Entries under one heading render in file-name order. In the
Unreleased section a heading with no entry is left out; in a released section
all three are written, and one with no entry says "Nothing.".

Python's standard library only.
"""
from __future__ import annotations

import argparse
import re
import shutil
import sys
import tempfile
from pathlib import Path

CHANGELOG = "crates/goatbuilder/CHANGELOG.md"
ENTRIES = "crates/goatbuilder/changelog.d"
HEADINGS = ("For someone deploying", "For someone contributing", "To be removed")
NOTHING = "Nothing."
BEGIN = f"<!-- BEGIN generated: changelog unreleased, from {ENTRIES}; edit the entries, not this section -->"
END = "<!-- END generated: changelog unreleased -->"
UNRELEASED = "## Unreleased"
SLUG = re.compile(r"^[a-z0-9][a-z0-9-]*\.md$")
VERSION = re.compile(r"^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$")
DATE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
# Files in the entries directory that are not entries.
NOT_ENTRIES = ("README.md",)


class Refused(Exception):
    """One or more reasons, each a line."""


def read_entry(path: Path) -> dict:
    """An entry's heading, title and body, or Refused naming what is wrong."""
    name = path.name
    errors = []
    if not SLUG.match(name):
        errors.append(f"{name}: the file name is not <slug>.md in lower case letters, digits and hyphens")
    lines = path.read_text(encoding="utf-8").split("\n")
    if not lines or lines[0] != "---":
        raise Refused(errors + [f"{name}: the file does not start with a `---` header"])
    try:
        close = lines.index("---", 1)
    except ValueError:
        raise Refused(errors + [f"{name}: the header has no closing `---` line"])
    fields = {}
    for line in lines[1:close]:
        key, sep, value = line.partition(":")
        key = key.strip()
        if not sep or key not in ("heading", "title"):
            errors.append(f"{name}: header line {line!r} is not `heading:` or `title:`")
            continue
        if key in fields:
            errors.append(f"{name}: `{key}` is given twice")
        fields[key] = value.strip()
    heading = fields.get("heading", "")
    title = fields.get("title", "")
    if heading not in HEADINGS:
        errors.append(f"{name}: heading {heading!r} is not one of: " + ", ".join(f'"{h}"' for h in HEADINGS))
    if not title:
        errors.append(f"{name}: no `title`")
    elif title.startswith("#"):
        errors.append(f"{name}: the title starts with `#`; give the words only")
    body = "\n".join(lines[close + 1:]).strip("\n")
    if not body.strip():
        errors.append(f"{name}: the body is empty")
    in_code = False
    for line in body.split("\n"):
        if line.lstrip().startswith("```"):
            in_code = not in_code
        elif not in_code and re.match(r"^#{1,3} ", line):
            errors.append(f"{name}: the body has a heading of level 1 to 3 ({line!r}); use #### or deeper")
    if errors:
        raise Refused(errors)
    return {"name": name, "heading": heading, "title": title, "body": body}


def read_entries(repo: Path) -> list[dict]:
    folder = repo / ENTRIES
    if not folder.is_dir():
        return []
    entries, errors = [], []
    for path in sorted(folder.iterdir()):
        if path.name in NOT_ENTRIES or path.is_dir():
            continue
        try:
            entries.append(read_entry(path))
        except Refused as e:
            errors.extend(e.args[0])
    if errors:
        raise Refused(errors)
    return entries


def render(entries: list[dict], every_heading: bool) -> list[str]:
    """The lines of the entries under their headings, with no blank line at either end."""
    out: list[str] = []
    for heading in HEADINGS:
        under = [e for e in entries if e["heading"] == heading]
        if not under and not every_heading:
            continue
        if out:
            out.append("")
        out += [f"### {heading}", ""]
        if not under:
            out.append(NOTHING)
        for i, e in enumerate(under):
            if i:
                out.append("")
            out += [f"#### {e['title']}", ""] + e["body"].split("\n")
    return out


def region(lines: list[str]) -> tuple[int, int]:
    """The indexes of the BEGIN and END lines, or Refused."""
    errors = []
    begins = [i for i, l in enumerate(lines) if l == BEGIN]
    ends = [i for i, l in enumerate(lines) if l == END]
    heads = [i for i, l in enumerate(lines) if l == UNRELEASED]
    if len(heads) != 1:
        errors.append(f"{CHANGELOG}: expected one `{UNRELEASED}` line, found {len(heads)}")
    if len(begins) != 1 or len(ends) != 1:
        errors.append(f"{CHANGELOG}: expected one BEGIN and one END marker line, found {len(begins)} and {len(ends)}")
    if errors:
        raise Refused(errors)
    head, b, e = heads[0], begins[0], ends[0]
    if not head < b < e:
        raise Refused([f"{CHANGELOG}: the markers are not under `{UNRELEASED}`, BEGIN before END"])
    if any(l.strip() for l in lines[head + 1:b]):
        errors.append(f"{CHANGELOG}: text between `{UNRELEASED}` and the BEGIN marker; put it in an entry")
    nxt = next((i for i in range(e + 1, len(lines)) if lines[i].startswith("## ")), len(lines))
    if any(l.strip() for l in lines[e + 1:nxt]):
        errors.append(f"{CHANGELOG}: text between the END marker and the next release; put it in an entry")
    if errors:
        raise Refused(errors)
    return b, e


def unreleased_lines(entries: list[dict]) -> list[str]:
    body = render(entries, every_heading=False)
    return [BEGIN] + ([""] + body + [""] if body else []) + [END]


def check(repo: Path) -> list[str]:
    """Every reason the changelog is not as rendered; empty when it is."""
    try:
        entries = read_entries(repo)
        lines = (repo / CHANGELOG).read_text(encoding="utf-8").split("\n")
        b, e = region(lines)
    except Refused as r:
        return list(r.args[0])
    if lines[b:e + 1] != unreleased_lines(entries):
        return [f"{CHANGELOG}: the Unreleased section differs from its render; "
                f"run `python3 ops/release/changelog.py write` (a hand edit there is lost: edit {ENTRIES}/ instead)"]
    return []


def write(repo: Path) -> None:
    entries = read_entries(repo)
    path = repo / CHANGELOG
    lines = path.read_text(encoding="utf-8").split("\n")
    b, e = region(lines)
    lines[b:e + 1] = unreleased_lines(entries)
    path.write_text("\n".join(lines), encoding="utf-8")


def release(repo: Path, version: str, date: str) -> list[Path]:
    """Fold the entries into a dated section; return the entry files removed."""
    errors = []
    if not VERSION.match(version):
        errors.append(f"version {version!r} is not X.Y.Z or X.Y.Z-<pre-release>")
    if not DATE.match(date):
        errors.append(f"date {date!r} is not YYYY-MM-DD")
    path = repo / CHANGELOG
    lines = path.read_text(encoding="utf-8").split("\n")
    if any(l == f"## {version}" or l.startswith(f"## {version} ") for l in lines):
        errors.append(f"{CHANGELOG} already has a section for {version}")
    if errors:
        raise Refused(errors)
    entries = read_entries(repo)
    b, e = region(lines)
    section = [f"## {version} — {date}", ""] + render(entries, every_heading=True)
    nxt = next((i for i in range(e + 1, len(lines)) if lines[i].startswith("## ")), None)
    after = lines[nxt:] if nxt is not None else []
    lines = lines[:b] + unreleased_lines([]) + [""] + section + ([""] + after if after else [""])
    path.write_text("\n".join(lines), encoding="utf-8")
    removed = []
    for entry in entries:
        p = repo / ENTRIES / entry["name"]
        p.unlink()
        removed.append(p)
    return removed


# --- self-test ---------------------------------------------------------------

def _copy(repo: Path, into: Path) -> Path:
    (into / ENTRIES).mkdir(parents=True)
    shutil.copy(repo / CHANGELOG, into / CHANGELOG)
    for p in (repo / ENTRIES).iterdir() if (repo / ENTRIES).is_dir() else []:
        if p.is_file():
            shutil.copy(p, into / ENTRIES / p.name)
    return into


def _entry(repo: Path, name: str, heading: str, title: str, body: str) -> None:
    (repo / ENTRIES / name).write_text(f"---\nheading: {heading}\ntitle: {title}\n---\n\n{body}\n", encoding="utf-8")


def self_test(repo: Path) -> int:
    failures = []

    def expect(what: str, ok: bool) -> None:
        print(f"  {'ok  ' if ok else 'FAIL'} {what}")
        if not ok:
            failures.append(what)

    print("changelog.py self-test, on a copy of the changelog and its entries:")
    with tempfile.TemporaryDirectory() as tmp:
        base = _copy(repo, Path(tmp) / "base")

        def fresh(name: str) -> Path:
            d = Path(tmp) / name
            shutil.copytree(base, d)
            return d

        expect("the copy passes the check", check(base) == [])

        r = fresh("hand-edit")
        p = r / CHANGELOG
        text = p.read_text(encoding="utf-8")
        p.write_text(text.replace(END, "A line written by hand.\n" + END, 1), encoding="utf-8")
        expect("a line written by hand in the Unreleased section is refused",
               any("differs from its render" in m for m in check(r)))

        r = fresh("outside")
        p = r / CHANGELOG
        p.write_text(p.read_text(encoding="utf-8").replace(END, END + "\n\nA stray line.", 1), encoding="utf-8")
        expect("a line between the END marker and the last release is refused",
               any("between the END marker" in m for m in check(r)))

        r = fresh("bad-heading")
        _entry(r, "bad-heading.md", "For someone testing", "Fixed: something", "A body.")
        expect("an entry whose heading is not one of the three is refused",
               any("bad-heading.md: heading" in m for m in check(r)))

        r = fresh("no-title")
        (r / ENTRIES / "no-title.md").write_text("---\nheading: To be removed\n---\n\nA body.\n", encoding="utf-8")
        expect("an entry with no title is refused", any("no-title.md: no `title`" in m for m in check(r)))

        r = fresh("level-3")
        _entry(r, "level-3.md", "For someone deploying", "Fixed: x", "### Not allowed\n\ntext")
        expect("an entry whose body has a level-3 heading is refused",
               any("level-3.md: the body has a heading" in m for m in check(r)))

        r = fresh("bad-name")
        _entry(r, "Bad_Name.md", "For someone deploying", "Fixed: x", "text")
        expect("an entry whose file name is not a slug is refused",
               any("Bad_Name.md: the file name" in m for m in check(r)))

        r = fresh("added")
        _entry(r, "zz-added.md", "For someone contributing", "Added: a check", "It checks.")
        expect("a new entry, before `write`, is refused", check(r) != [])
        write(r)
        expect("after `write`, the check passes", check(r) == [])
        text = (r / CHANGELOG).read_text(encoding="utf-8")
        unreleased = text[text.index(BEGIN):text.index(END)]
        under = unreleased.split("### For someone contributing\n", 1)[-1].split("\n### ", 1)[0]
        expect("`write` renders the new entry under its heading, whatever entries the repository holds",
               "### For someone contributing\n" in unreleased and "#### Added: a check\n\nIt checks." in under)

        before = (r / CHANGELOG).read_text(encoding="utf-8")
        released_before = before[before.index(END) + len(END):]
        removed = release(r, "9.9.9", "2099-01-02")
        after = (r / CHANGELOG).read_text(encoding="utf-8")
        expect("the release folds every entry and removes its file",
               bool(removed) and not any(p.exists() for p in removed))
        expect("the release keeps the entries directory's README",
               not (base / ENTRIES / "README.md").exists() or (r / ENTRIES / "README.md").exists())
        expect("the release's section follows the empty Unreleased section",
               f"{BEGIN}\n{END}\n\n## 9.9.9 — 2099-01-02\n\n### For someone deploying" in after)
        expect("the release writes all three headings, and \"Nothing.\" under an empty one",
               all(f"### {h}" in after.split("## 9.9.9")[1] for h in HEADINGS)
               and "### To be removed\n\nNothing." in after)
        expect("the sections released before are unchanged, byte for byte", after.endswith(released_before))
        expect("after the release, the check passes", check(r) == [])
        try:
            release(r, "9.9.9", "2099-01-03")
            expect("a second release of the same version is refused", False)
        except Refused:
            expect("a second release of the same version is refused", True)
        write(r)
        expect("`write` after the release leaves the released section alone",
               (r / CHANGELOG).read_text(encoding="utf-8") == after)
        try:
            release(r, "9.10", "2099-13")
            expect("a malformed version and date are refused", False)
        except Refused as e:
            expect("a malformed version and date are refused", len(e.args[0]) == 2)

    print("the check, on this repository:")
    problems = check(repo)
    for m in problems:
        print(f"  {m}")
    expect("this repository's changelog equals its render", problems == [])
    if failures:
        print(f"self-test FAILED: {len(failures)} case(s)")
        return 1
    print("self-test passed")
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2])
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("command", nargs="?", choices=("check", "write", "release"))
    ap.add_argument("args", nargs="*")
    a = ap.parse_args(argv)
    if a.self_test:
        return self_test(a.repo)
    try:
        if a.command in (None, "check"):
            problems = check(a.repo)
            for m in problems:
                print(f"changelog: {m}", file=sys.stderr)
            if not problems:
                print(f"changelog: ok, the Unreleased section equals its render from {ENTRIES}/")
            return 1 if problems else 0
        if a.command == "write":
            write(a.repo)
            print(f"changelog: wrote the Unreleased section from {ENTRIES}/")
            return 0
        if len(a.args) != 2:
            ap.error("release takes <version> <date>, for example: release 0.7.0 2026-11-20")
        removed = release(a.repo, *a.args)
        print(f"changelog: folded {len(removed)} entr{'y' if len(removed) == 1 else 'ies'} into "
              f"## {a.args[0]} — {a.args[1]}, and removed them:")
        for p in removed:
            print(f"  {p.relative_to(a.repo)}")
        return 0
    except Refused as r:
        for m in r.args[0]:
            print(f"changelog: {m}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
