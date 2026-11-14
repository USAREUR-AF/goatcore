#!/usr/bin/env python3
"""changelog: goatbuilder's changelog, rendered from one file per entry (ADR 1231).

An entry is a file in crates/goatbuilder/changelog.d/, so a pull request that
adds one edits no shared file. CHANGELOG.md's "Unreleased" section is rendered
from those files, between two marker lines. A release folds them into a dated
section and removes them. A released section is never rendered again, and is
never edited after its release: the check holds it to its text at the tag
goatbuilder/v<version>, or, before the tag exists, at the release commit (the
newest `chore(release): goatbuilder <version>` commit on HEAD's first-parent
history). A release-notes change is itself such a commit. A shallow clone, or a
version with neither, is skipped with a note.

  changelog.py check                     exit 0 when every entry is well formed, none is
                                         already in a released section, every released
                                         section is as it was at its release, and the
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
import subprocess
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


def released_sections(lines: list[str], end: int) -> list[tuple[str, list[str]]]:
    """Each released section after the END marker, as (version, its lines)."""
    starts = [i for i in range(end + 1, len(lines)) if lines[i].startswith("## ")]
    out = []
    for n, i in enumerate(starts):
        stop = starts[n + 1] if n + 1 < len(starts) else len(lines)
        out.append((lines[i][3:].split(" ", 1)[0], lines[i + 1:stop]))
    return out


def _holds(lines: list[str], block: list[str]) -> bool:
    k = len(block)
    return any(lines[i:i + k] == block for i in range(len(lines) - k + 1))


def already_released(entries: list[dict], lines: list[str], end: int,
                     at: dict[str, tuple[str, str, list[str]]] | None = None) -> list[str]:
    """An entry whose rendered text (its title line, a blank line and its body, the whole
    block: a title alone can repeat between releases) is inside a released section.
    When that section's text at its release (`at`, from _release_texts) does not hold
    it, the entry landed there after the release: a branch cut before the release and
    merged after it. The section is restored and the entry kept. Otherwise it was
    released and its file was left behind: the file goes."""
    sections = released_sections(lines, end)
    problems = []
    for e in entries:
        block = [f"#### {e['title']}", ""] + e["body"].split("\n")
        for version, body in sections:
            if not _holds(body, block):
                continue
            if at and version in at and not _holds(at[version][2], block):
                ref, named, _ = at[version]
                problems.append(f"{ENTRIES}/{e['name']}: its text landed inside {version} after its release (it is not "
                                f"in {version} at {named}): restore that section from `git show {_short(ref)}:{CHANGELOG}`, "
                                f"keep the entry, then run `python3 ops/release/changelog.py write`")
            else:
                problems.append(f"{ENTRIES}/{e['name']}: already released in {version}; remove the entry file "
                                f"(a release folds an entry and removes its file)")
            break
    return problems


def _short(ref: str) -> str:
    """A commit id cut to 12 characters; a tag as it is."""
    return ref[:12] if re.fullmatch(r"[0-9a-f]{40}", ref) else ref


def _git_out(repo: Path, *args: str) -> str | None:
    """git's output in repo, or None when git fails (not a checkout, no such object)."""
    try:
        r = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return r.stdout if r.returncode == 0 else None


def _release_ref(repo: Path, version: str) -> tuple[str, str] | None:
    """Where version's released text is: the tag goatbuilder/v<version>, or, before the
    tag exists, the newest commit on HEAD's first-parent history whose subject begins
    `chore(release): goatbuilder <version>` (the release, or a later release-notes
    change, which uses the same subject). As (ref, how it is named), or None."""
    tag = f"goatbuilder/v{version}"
    if _git_out(repo, "rev-parse", "--verify", "-q", f"refs/tags/{tag}^{{commit}}"):
        return tag, f"the tag {tag}"
    log = _git_out(repo, "log", "--first-parent", "--format=%H %s", "HEAD") or ""
    for line in log.splitlines():
        sha, _, subject = line.partition(" ")
        rest = subject[len(f"chore(release): goatbuilder {version}"):]
        if subject.startswith(f"chore(release): goatbuilder {version}") and (not rest or not rest[0].isalnum() and rest[0] != "."):
            return sha, f"the release commit {sha[:8]} (no tag {tag} here)"
    return None


def _release_texts(repo: Path, lines: list[str]) -> tuple[dict[str, tuple[str, str, list[str]]], list[str]]:
    """Each released section's text at its release, as {version: (ref, how it is named,
    its lines)}, and a note for each version whose release cannot be read here."""
    at: dict[str, tuple[str, str, list[str]]] = {}
    notes: list[str] = []
    try:
        _, end = region(lines)
    except Refused:
        return at, notes
    sections = released_sections(lines, end)
    if not sections:
        return at, notes
    shallow = _git_out(repo, "rev-parse", "--is-shallow-repository")
    if shallow is None:
        notes.append("released sections not compared with their release: not a git checkout; skipped")
        return at, notes
    shallow = shallow.strip() == "true"
    for version, _ in sections:
        where = _release_ref(repo, version)
        if where is None:
            notes.append(f"{version}: no tag goatbuilder/v{version} and no `chore(release): goatbuilder {version}` "
                         f"commit on this history" + (" (a shallow clone: its release may be cut off; CI checks "
                                                       "out with fetch-depth: 0)" if shallow else "") + "; skipped")
            continue
        ref, named = where
        then = _git_out(repo, "show", f"{ref}:{CHANGELOG}")
        if then is None:
            notes.append(f"{version}: {CHANGELOG} cannot be read at {named}; skipped")
            continue
        old = then.split("\n")
        try:
            _, old_end = region(old)
        except Refused:
            old_end = -1
        released = dict(released_sections(old, old_end))
        if version not in released:
            notes.append(f"{version}: {named} has no `## {version}` section; skipped")
            continue
        at[version] = (ref, named, released[version])
    return at, notes


def released_text(repo: Path, lines: list[str],
                  at_notes: tuple[dict, list[str]] | None = None) -> tuple[list[str], list[str]]:
    """A released section is never edited after its release (a release-notes change is
    itself a `chore(release)` commit, or moves the tag): each is compared with its text
    at its release, the release commit's text as it was committed, not a re-render. As
    (problems, notes); a version whose release cannot be read is skipped with a note,
    never passed silently."""
    at, notes = at_notes if at_notes is not None else _release_texts(repo, lines)
    problems: list[str] = []
    try:
        _, end = region(lines)
    except Refused:
        return problems, notes
    for version, body in released_sections(lines, end):
        if version not in at:
            continue
        ref, named, then = at[version]
        if [l.rstrip() for l in body] != [l.rstrip() for l in then] and \
                [l for l in body if l.strip()] != [l for l in then if l.strip()]:
            added = [l for l in body if l.strip() and l not in then]
            gone = [l for l in then if l.strip() and l not in body]
            what = []
            if added:
                what.append("added: " + " | ".join(added[:3]) + (" …" if len(added) > 3 else ""))
            if gone:
                what.append("removed: " + " | ".join(gone[:3]) + (" …" if len(gone) > 3 else ""))
            problems.append(f"{CHANGELOG}: the released section {version} differs from its text at {named} "
                            f"({'; '.join(what) or 'lines moved'}). A released section is never edited after its "
                            f"release: restore it from `git show {_short(ref)}:{CHANGELOG}`; an entry merged after the "
                            f"release stays in {ENTRIES}/ and `write` renders it under Unreleased")
    return problems, notes


def check(repo: Path) -> list[str]:
    """Every reason the changelog is not as rendered; empty when it is."""
    try:
        entries = read_entries(repo)
        lines = (repo / CHANGELOG).read_text(encoding="utf-8").split("\n")
        b, e = region(lines)
    except Refused as r:
        return list(r.args[0])
    at_notes = _release_texts(repo, lines)
    # The released-entry reasons first: following the generic `write` one alone would
    # render such an entry a second time, under Unreleased.
    problems = already_released(entries, lines, e, at_notes[0]) + released_text(repo, lines, at_notes)[0]
    if lines[b:e + 1] != unreleased_lines(entries):
        problems.append(f"{CHANGELOG}: the Unreleased section differs from its render; "
                        f"run `python3 ops/release/changelog.py write` (a hand edit there is lost: edit {ENTRIES}/ instead)")
    return problems


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


def _git(repo: Path, *args: str) -> str:
    return subprocess.run(["git", "-C", str(repo), "-c", "user.name=self-test", "-c", "user.email=self-test@example.org",
                           "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", *args],
                          check=True, capture_output=True, text=True).stdout


def _released_text_cases(expect) -> None:
    """A released section is never edited after its release: it is held to its text at
    the tag goatbuilder/v<version>, or, before the tag exists, at the release commit."""
    with tempfile.TemporaryDirectory() as tmp:
        r = Path(tmp) / "repo"
        (r / ENTRIES).mkdir(parents=True)
        _git(r.parent, "init", "-q", "-b", "main", str(r))
        cl = r / CHANGELOG
        cl.write_text("# Changelog\n\n" + UNRELEASED + "\n\n" + BEGIN + "\n" + END + "\n\n## 9.9.9 — 2099-01-02\n\n"
                      "### For someone deploying\n\n#### Fixed: a thing\n\nIt was fixed.\n", encoding="utf-8")
        _git(r, "add", "-A")
        _git(r, "commit", "-q", "-m", "chore(release): goatbuilder 9.9.9")
        expect("a released section as it was at its release commit passes", check(r) == [])

        text = cl.read_text(encoding="utf-8")
        cl.write_text(text + "\n#### Fixed: a thing merged after the release\n\nIt landed late.\n", encoding="utf-8")
        _git(r, "commit", "-q", "-am", "feat: a branch cut before the release, merged after it")
        problems = check(r)
        expect("a line added to a released section after its release commit is refused, naming the version",
               any("9.9.9" in m and "release commit" in m and "merged after the release" in m for m in problems))
        _entry(r, "late.md", "For someone deploying", "Fixed: a thing merged after the release", "It landed late.")
        problems = check(r)
        late = [m for m in problems if "late.md" in m]
        expect("an entry that landed inside a release after it is told to restore the section and keep the entry",
               len(late) == 1 and "landed inside 9.9.9 after its release" in late[0] and "restore" in late[0]
               and "remove the entry file" not in late[0])
        expect("and that reason comes before the generic `write` one",
               bool(late) and problems.index(late[0]) < next((i for i, m in enumerate(problems) if "differs from its render" in m), len(problems)))
        (r / ENTRIES / "late.md").unlink()

        # A release-notes change: the edit, and its line in the baseline, in one change.
        (r / BASELINE).parent.mkdir(parents=True, exist_ok=True)
        edited = cl.read_text(encoding="utf-8").split("\n")
        _, end = region(edited)
        section = dict(released_sections(edited, end))["9.9.9"]
        (r / BASELINE).write_text(f"9.9.9 {section_hash(section)}\n", encoding="utf-8")
        expect("a released section edited after its release passes with its line in the baseline", check(r) == [])
        (r / BASELINE).write_text("9.9.9 sha256:" + "0" * 64 + "\n", encoding="utf-8")
        expect("a baseline line whose hash is not the section's is refused, naming the version",
               any("9.9.9" in m and BASELINE in m and "hash" in m for m in check(r)))
        (r / BASELINE).unlink()

        cl.write_text(text, encoding="utf-8")
        _git(r, "commit", "-q", "-am", "fix: put 9.9.9 back as released")
        (r / BASELINE).write_text(f"9.9.9 {section_hash(dict(released_sections(text.split(chr(10)), region(text.split(chr(10)))[1]))['9.9.9'])}\n", encoding="utf-8")
        expect("a baseline line for a section that is as released is refused as stale (the baseline only shrinks)",
               any("9.9.9" in m and "stale" in m for m in check(r)))
        (r / BASELINE).unlink()
        _git(r, "tag", "goatbuilder/v9.9.9", "HEAD~2")
        cl.write_text(text.replace("It was fixed.", "It was fixed, then reworded."), encoding="utf-8")
        _git(r, "commit", "-q", "-am", "docs: reword a released line")
        expect("with the tag, a released section is held to its text at the tag",
               any("9.9.9" in m and "goatbuilder/v9.9.9" in m for m in check(r)))

        cl.write_text(text.replace("## 9.9.9", "## 9.9.10 — 2099-02-03\n\n### For someone deploying\n\nNothing.\n\n## 9.9.9"), encoding="utf-8")
        _git(r, "commit", "-q", "-am", "a section with neither a tag nor a release commit")
        problems, notes = released_text(r, cl.read_text(encoding="utf-8").split("\n"))
        expect("a version with neither a tag nor a release commit is skipped with a note, not refused",
               not any("9.9.10" in m for m in problems) and any("9.9.10" in n and "skipped" in n for n in notes))

        shallow = Path(tmp) / "shallow"
        _git(Path(tmp), "clone", "-q", "--depth", "1", f"file://{r}", str(shallow))
        (shallow / ENTRIES).mkdir(parents=True, exist_ok=True)
        problems, notes = released_text(shallow, (shallow / CHANGELOG).read_text(encoding="utf-8").split("\n"))
        expect("in a shallow clone every released section is skipped with a note, never passed silently",
               problems == [] and any("shallow" in n for n in notes))


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

        # An entry whose text is already in a released section: a branch cut before a
        # release, merged after it, leaves its rendered lines inside the release and its
        # entry file behind (both #753 and #671 did so on 14 Nov, after 0.7.0's fold).
        r = fresh("released-twice")
        _entry(r, "zz-twice.md", "For someone deploying", "Fixed: a thing released once", "It was fixed.\n\n- one\n- two")
        write(r)
        release(r, "9.8.7", "2099-01-01")
        _entry(r, "zz-twice.md", "For someone deploying", "Fixed: a thing released once", "It was fixed.\n\n- one\n- two")
        problems = check(r)
        expect("an entry whose text is already in a released section is refused, naming the release",
               any("zz-twice.md: already released in 9.8.7" in m for m in problems))
        write(r)
        expect("and `write` does not make it pass", any("already released in 9.8.7" in m for m in check(r)))
        r2 = fresh("same-title")
        _entry(r2, "zz-twice.md", "For someone deploying", "Fixed: a thing released once", "It was fixed.\n\n- one\n- two")
        write(r2)
        release(r2, "9.8.7", "2099-01-01")
        _entry(r2, "zz-again.md", "For someone deploying", "Fixed: a thing released once", "Fixed again, differently.")
        write(r2)
        expect("an entry with a released title but another body is not refused", check(r2) == [])

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

    print("released sections against their release text (a git history of its own):")
    _released_text_cases(expect)

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
            try:
                for n in released_text(a.repo, (a.repo / CHANGELOG).read_text(encoding="utf-8").split("\n"))[1]:
                    print(f"changelog: note: {n}")
            except OSError:
                pass
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
