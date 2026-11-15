#!/usr/bin/env python3
"""check-drill-log-coded.py — a drill log holds no error without its GB-nnn code.

Gate row C5 (docs/design/goatbuilder-v1-gate.md): "The drill logs contain no
error without a GB-nnn code." A drill (row C4) keeps what goatbuilder printed:
the terminal output, as text, and the `-o json` reports. This reads them and
names every error that carries no code.

An error, in a text log, is:
  - goatbuilder's refusal line, `goatbuilder: <text>`, which main.rs prints when
    stderr is not a terminal;
  - a usage error, a line starting `error: ` (clap's), which goatbuilder codes
    GB-242;
  - a verb's failed row: `failed  !! …` as the terminal renders it (the row can
    wrap onto the lines after it), `!! … FAILED …`, or `… FAILED — exit N`.
Colour codes are removed first. It is coded when a GB-nnn code is on its line
or on one of the next twelve (a failed row wraps, and its code is on the
`interpreted` row under it). `uncatalogued` is no
code. The runner's own row for each command, `FAILED: exit N in <time>; log
<path>`, is not read: a probe that fails and is gone past prints one too, and a
failure that stops the verb prints the verb's failed row as well.

In a JSON report (an object with a `checks` list, or one per line), an error is
a row named `failed`, `refused` or `error` with `ok` false; it is coded when its
detail or fix, or the row after it, carries a code. Rows that report a state
(a host pending, drift found) are verdicts, not errors, and are not read.

usage: check-drill-log-coded.py <file or directory>... | --self-test
exit 0 every error coded · 1 an error without a code (each named, file:line) · 2 cannot read
"""
import json
import re
import sys
from pathlib import Path

CODE = re.compile(r"\bGB-\d{3}\b")
REFUSAL = re.compile(r"^\s*goatbuilder: ")
USAGE = re.compile(r"^\s*error: ")
FAILED = re.compile(r"\bfailed\s+!!|!!.*\bFAILED\b|\bFAILED — exit \d+")
ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
WINDOW = 12
ERROR_ROWS = {"failed", "refused", "error"}


def files_of(paths):
    out = []
    for p in paths:
        p = Path(p)
        if p.is_dir():
            out.extend(sorted(f for f in p.rglob("*") if f.is_file()))
        elif p.is_file():
            out.append(p)
        else:
            raise FileNotFoundError(str(p))
    return out


def json_docs(text):
    """The reports in `text`: one JSON document, or one per line. None when it is not JSON."""
    s = text.strip()
    if not s or s[0] not in "{[":
        return None
    try:
        doc = json.loads(s)
        return doc if isinstance(doc, list) else [doc]
    except json.JSONDecodeError:
        pass
    docs = []
    for line in s.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            docs.append(json.loads(line))
        except json.JSONDecodeError:
            return None
    return docs


def uncoded_json(path, docs):
    found = []
    for n, doc in enumerate(docs, 1):
        if not isinstance(doc, dict):
            continue
        rows = doc.get("checks")
        if not isinstance(rows, list):
            continue
        for i, row in enumerate(rows):
            if not isinstance(row, dict) or row.get("ok", True) or row.get("name") not in ERROR_ROWS:
                continue
            said = f"{row.get('detail', '')} {row.get('fix', '') or ''}"
            nxt = rows[i + 1] if i + 1 < len(rows) and isinstance(rows[i + 1], dict) else {}
            said_next = f"{nxt.get('detail', '')} {nxt.get('fix', '') or ''}"
            if not (CODE.search(said) or CODE.search(said_next)):
                found.append((str(path), n, f"{row.get('name')}: {row.get('detail', '')}"))
    return found


def uncoded_text(path, text):
    lines = [ANSI.sub("", l) for l in text.splitlines()]
    found = []
    for i, line in enumerate(lines):
        if not (REFUSAL.search(line) or USAGE.search(line) or FAILED.search(line)):
            continue
        window = "\n".join(lines[i:i + 1 + WINDOW])
        if CODE.search(window):
            continue
        found.append((str(path), i + 1, line.strip()))
    return found


def uncoded(paths):
    """Every error without a code in `paths`: (file, line, the error's text)."""
    found = []
    for f in files_of(paths):
        text = f.read_text(errors="replace")
        docs = json_docs(text)
        found.extend(uncoded_json(f, docs) if docs is not None else uncoded_text(f, text))
    return found


def self_test():
    import unittest
    here = Path(__file__).resolve().parent
    suite = unittest.defaultTestLoader.discover(str(here), pattern="test_check_drill_log_coded.py")
    result = unittest.TextTestRunner(verbosity=1).run(suite)
    return 0 if result.wasSuccessful() else 1


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    if not argv:
        print(__doc__.strip().splitlines()[-2], file=sys.stderr)
        return 2
    try:
        found = uncoded(argv)
    except OSError as e:
        print(f"check-drill-log-coded: cannot read {e}", file=sys.stderr)
        return 2
    for f, n, what in found:
        print(f"{f}:{n}: no GB-nnn code: {what}")
    if found:
        print(f"check-drill-log-coded: {len(found)} error(s) without a code")
        return 1
    print("check-drill-log-coded: ok, every error carries its code")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
