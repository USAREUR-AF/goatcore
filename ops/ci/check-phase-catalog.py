#!/usr/bin/env python3
"""Check the phase catalog (ops/phases/catalog.yaml), and that the documents'
phase tables are the ones rendered from it.

  * every entry has a verb, a phase, and the fields `explain` answers from
    (what, why, changes, verify, runs, by_hand, read), each non-empty;
  * no verb has a phase twice, and every `read` path exists;
  * each generated table in a document, between
        <!-- BEGIN generated: phase-catalog <verb> <table> -->
        <!-- END generated: phase-catalog <verb> <table> -->
    equals what this script renders from the catalog.

usage: check-phase-catalog.py [--repo DIR] [--write | --self-test]

--write rewrites every generated table from the catalog. The catalog is the
one place the text is edited; a table edited by hand fails this check.
--self-test proves the check refuses a hand-edited table, a missing field and
a missing `read` path on a copy, then runs the check on the repository.
"""
import argparse
import os
import re
import sys

import yaml

CATALOG = "ops/phases/catalog.yaml"
SCHEMA = "goat-phases/v1"
REQUIRED = ("what", "why", "changes", "verify", "runs", "by_hand", "read")

# The documents carrying generated tables. A table is named by its marker.
DOCUMENTS = (
    "docs/operations/sovereign-federation-runbook.md",
    "docs/operations/goatnet-standup.md",
    "docs/reference/goatbuilder/10-sites.md",
)

BEGIN = re.compile(r"^<!-- BEGIN generated: phase-catalog (\S+) (\S+) -->$")
END = "<!-- END generated: phase-catalog {verb} {table} -->"


def cell(text):
    """One table cell: a single line, with any pipe escaped."""
    return " ".join(str(text).split()).replace("|", "\\|")


def kind_cell(kind):
    # A stop for a person is in bold, as the runbook has always shown it.
    return f"**{kind}**" if kind == "CONFIRM" else kind


def order(e):
    """Numbered phases in order, then the named steps in the catalog's order."""
    return (0, e["phase"]) if isinstance(e["phase"], int) else (1, 0)


def rows(entries, verb, stage=None):
    out = [e for e in entries if e["verb"] == verb and (stage is None or e.get("stage") == stage)]
    return sorted(out, key=order)


def run_rows(entries, verb):
    """The numbered phases, then the named steps that run beside them. A join
    is where the run waits for a step, not a step of its own: it has no row."""
    return [e for e in rows(entries, verb) if not str(e["phase"]).startswith("join-")]


# Each table: its header, and how one entry becomes one row.
TABLES = {
    "plan": (
        "| # | Kind | Phase | Worked when |\n|---|---|---|---|",
        lambda es, v: [
            f"| {e['phase']} | {kind_cell(e['kind'])} | {cell(e['title'])} | {cell(e['verify'])} |"
            for e in rows(es, v, "plan")
        ],
    ),
    "deploy": (
        "| # | Phase | What `deploy` does | Worked when |\n|---|---|---|---|",
        lambda es, v: [
            f"| {e['phase']} | {cell(e['title'])} | {cell(e['what'])} | {cell(e['verify'])} |"
            for e in rows(es, v, "deploy")
        ],
    ),
    # A verb's whole run: each numbered phase, then each step that runs beside them.
    "run": (
        "| # | Kind | Phase | What it does | Worked when |\n|---|---|---|---|---|",
        lambda es, v: [
            f"| {e['phase']} | {kind_cell(e['kind'])} | {cell(e['title'])} | {cell(e['what'])} | {cell(e['verify'])} |"
            for e in run_rows(es, v)
        ],
    ),
    # One line a phase, for a chapter that lists a verb's phases.
    "summary": (
        "| Phase | Kind | What it does |\n|---|---|---|",
        lambda es, v: [
            f"| {e['phase']} | {kind_cell(e['kind'])} | {cell(e['title'])} |"
            for e in run_rows(es, v)
        ],
    ),
    "failures": (
        "| Phase | It failed because | What to do |\n|---|---|---|",
        lambda es, v: [
            f"| {e['phase']} | {cell(e['failed_because'])} | {cell(e['then'])} |"
            for e in rows(es, v)
        ],
    ),
}


def render(entries, verb, table):
    """The lines between the markers: the table, with a blank line on each side
    so that Markdown never reads a marker as part of the table."""
    header, body = TABLES[table]
    return "\n".join(["", header, *body(entries, verb), ""])


def load(repo):
    path = os.path.join(repo, CATALOG)
    with open(path, encoding="utf-8") as f:
        doc = yaml.safe_load(f)
    problems = []
    if not isinstance(doc, dict) or doc.get("schema") != SCHEMA:
        return [], [f"{CATALOG}: schema is not {SCHEMA}"]
    entries = doc.get("phases") or []
    seen = set()
    for i, e in enumerate(entries):
        where = f"{CATALOG}: entry {i + 1}"
        if not isinstance(e, dict) or not e.get("verb") or e.get("phase") in (None, ""):
            problems.append(f"{where}: needs a verb and a phase")
            continue
        key = (e["verb"], str(e["phase"]))
        where = f"{CATALOG}: {e['verb']} phase {e['phase']}"
        if key in seen:
            problems.append(f"{where}: appears twice")
        seen.add(key)
        for f in REQUIRED:
            if not str(e.get(f) or "").strip():
                problems.append(f"{where}: no `{f}`")
        read = str(e.get("read") or "").split("#")[0].strip()
        if read and not os.path.exists(os.path.join(repo, read)):
            problems.append(f"{where}: its `read` {read} does not exist")
    return entries, problems


def tables_in(text):
    """Every generated table: (verb, table, start line, end line), 0-based, markers included."""
    lines = text.split("\n")
    found = []
    i = 0
    while i < len(lines):
        m = BEGIN.match(lines[i])
        if m:
            verb, table = m.group(1), m.group(2)
            end = END.format(verb=verb, table=table)
            j = next((k for k in range(i + 1, len(lines)) if lines[k] == end), None)
            if j is None:
                raise ValueError(f"no `{end}` after line {i + 1}")
            found.append((verb, table, i, j))
            i = j
        i += 1
    return found


def self_test(repo):
    """Each refusal on a temporary copy of the catalog and the documents."""
    import shutil
    import subprocess
    import tempfile

    def copy(tmp):
        for rel in (CATALOG, *DOCUMENTS):
            os.makedirs(os.path.join(tmp, os.path.dirname(rel)), exist_ok=True)
            shutil.copy(os.path.join(repo, rel), os.path.join(tmp, rel))
        for e in yaml.safe_load(open(os.path.join(repo, CATALOG)))["phases"]:
            rel = str(e.get("read") or "").split("#")[0]
            if rel and not os.path.exists(os.path.join(tmp, rel)):
                os.makedirs(os.path.join(tmp, os.path.dirname(rel)), exist_ok=True)
                shutil.copy(os.path.join(repo, rel), os.path.join(tmp, rel))

    def run(tmp):
        return subprocess.run([sys.executable, os.path.abspath(__file__), "--repo", tmp],
                              capture_output=True, text=True)

    def edit(tmp, rel, old, new):
        path = os.path.join(tmp, rel)
        text = open(path, encoding="utf-8").read()
        assert old in text, (rel, old)
        open(path, "w", encoding="utf-8").write(text.replace(old, new, 1))

    cases = [
        ("a table edited by hand", DOCUMENTS[0], "| 4 | T | `lint` |", "| 4 | T | `lint`, edited |", "is not the one rendered"),
        ("an entry with no `why`", CATALOG, "  why: ", "  why_not: ", "no `why`"),
        ("a `read` path that does not exist", CATALOG, "read: docs/operations/sovereign-federation-runbook.md", "read: docs/operations/no-such-runbook.md", "does not exist"),
    ]
    failed = 0
    with tempfile.TemporaryDirectory() as tmp:
        copy(tmp)
        r = run(tmp)
        if r.returncode != 0:
            print(f"self-test: the unedited copy fails: {r.stderr}", file=sys.stderr)
            failed += 1
    for name, rel, old, new, want in cases:
        with tempfile.TemporaryDirectory() as tmp:
            copy(tmp)
            edit(tmp, rel, old, new)
            r = run(tmp)
            ok = r.returncode != 0 and want in r.stderr
            print(f"self-test: {name}: {'refused' if ok else 'NOT REFUSED'}")
            failed += 0 if ok else 1
    return failed


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--repo", default=os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")))
    ap.add_argument("--write", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test and self_test(a.repo):
        return 1
    entries, problems = load(a.repo)
    for doc in DOCUMENTS:
        path = os.path.join(a.repo, doc)
        with open(path, encoding="utf-8") as f:
            text = f.read()
        try:
            found = tables_in(text)
        except ValueError as e:
            problems.append(f"{doc}: {e}")
            continue
        if not found:
            problems.append(f"{doc}: has no generated phase table (BEGIN generated: phase-catalog <verb> <table>)")
            continue
        lines = text.split("\n")
        for verb, table, i, j in reversed(found):
            if table not in TABLES:
                problems.append(f"{doc}: line {i + 1}: no table named {table}")
                continue
            want = render(entries, verb, table).split("\n")
            if not rows(entries, verb):
                problems.append(f"{doc}: line {i + 1}: the catalog has no phase of {verb}")
            if lines[i + 1:j] != want:
                if a.write:
                    lines[i + 1:j] = want
                else:
                    problems.append(f"{doc}: the {verb} {table} table (line {i + 1}) is not the one rendered from {CATALOG}; run python3 ops/ci/check-phase-catalog.py --write")
        if a.write:
            new = "\n".join(lines)
            if new != text:
                with open(path, "w", encoding="utf-8") as f:
                    f.write(new)
                print(f"check-phase-catalog: wrote {doc}")
    for p in problems:
        print(p, file=sys.stderr)
    if problems:
        print(f"check-phase-catalog: {len(problems)} problem(s)", file=sys.stderr)
        return 1
    print(f"check-phase-catalog: OK ({len(entries)} phases, generated tables match)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
