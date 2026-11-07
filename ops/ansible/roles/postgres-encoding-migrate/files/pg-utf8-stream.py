#!/usr/bin/env python3
"""pg-utf8-stream.py — check or convert a plain-text Postgres dump stream to UTF-8.

Reads a plain SQL dump (pg_dump -Fp, pg_dumpall --globals-only, or
pg_restore -f -) on stdin. Two modes:

  scan    Read the whole stream and report every place whose bytes are not
          valid UTF-8, grouped by table (rows of a COPY block) or by "<ddl>"
          (statements outside COPY data). Prints one JSON object on stdout.
          Exit 0 when every byte is valid UTF-8, 3 when any is not.

  filter  Copy the stream to stdout for a restore into a UTF8 cluster:
            - outside COPY data, `SET client_encoding = 'SQL_ASCII';` becomes
              'UTF8' and a `CREATE DATABASE ... ENCODING = 'SQL_ASCII'` becomes
              ENCODING = 'UTF8', so the server validates the bytes as UTF-8;
            - with --repair latin1, every byte that is not part of a valid
              UTF-8 sequence is read as a LATIN1 character and written as its
              UTF-8 encoding; valid UTF-8 sequences are left as they are;
            - without --repair, the first invalid byte stops the stream with
              exit 3 (the restore fails closed rather than load altered text).

The dump format is line-based: COPY data escapes newlines inside values, so
one data line is one row.
"""

import argparse
import codecs
import json
import re
import sys

COPY_START = re.compile(rb"^COPY (\S+)(?: \(.*\))? FROM stdin;\s*$")
COPY_END = b"\\.\n"
SET_ENC = re.compile(rb"^SET client_encoding = 'SQL_ASCII';(\r?\n)$")
CREATE_DB = re.compile(rb"^(CREATE DATABASE .* ENCODING = )'SQL_ASCII'(.*)$", re.S)
MAX_SAMPLES = 5


def latin1_fallback(err):
    bad = err.object[err.start:err.end]
    return (bad.decode("latin-1"), err.end)


codecs.register_error("goat_latin1_fallback", latin1_fallback)


def first_bad(line):
    try:
        line.decode("utf-8")
        return None
    except UnicodeDecodeError as e:
        return e.start


def sample(line, pos, row):
    lo = max(0, pos - 16)
    hi = min(len(line), pos + 16)
    # The first field of a COPY row is usually the primary key; give enough of
    # it to find the row, as text with the bad bytes escaped.
    first = line.split(b"\t", 1)[0][:64]
    return {
        "row": row,
        "first_field": first.decode("utf-8", "backslashreplace"),
        "byte_offset": pos,
        "context_hex": line[lo:hi].hex(),
    }


def scan(stream, label):
    bad = {}
    table = None
    row = 0
    lineno = 0
    for line in stream:
        lineno += 1
        if table is not None:
            if line == COPY_END or line == b"\\.\r\n":
                table = None
                continue
            row += 1
            pos = first_bad(line)
            if pos is not None:
                ent = bad.setdefault(table, {"rows": 0, "samples": []})
                ent["rows"] += 1
                if len(ent["samples"]) < MAX_SAMPLES:
                    ent["samples"].append(sample(line, pos, row))
            continue
        m = COPY_START.match(line)
        if m:
            table = m.group(1).decode("utf-8", "backslashreplace")
            row = 0
        pos = first_bad(line)
        if pos is not None:
            ent = bad.setdefault("<ddl>", {"rows": 0, "samples": []})
            ent["rows"] += 1
            if len(ent["samples"]) < MAX_SAMPLES:
                s = sample(line, pos, lineno)
                s["line"] = s.pop("row")
                ent["samples"].append(s)
    out = {"source": label, "ok": not bad, "invalid": bad}
    print(json.dumps(out, sort_keys=True))
    return 0 if not bad else 3


def filt(stream, out, repair):
    in_copy = False
    lineno = 0
    for line in stream:
        lineno += 1
        if in_copy:
            if line == COPY_END or line == b"\\.\r\n":
                in_copy = False
        else:
            if COPY_START.match(line):
                in_copy = True
            else:
                m = SET_ENC.match(line)
                if m:
                    line = b"SET client_encoding = 'UTF8';" + m.group(1)
                else:
                    m = CREATE_DB.match(line)
                    if m:
                        line = m.group(1) + b"'UTF8'" + m.group(2)
        pos = first_bad(line)
        if pos is not None:
            if repair != "latin1":
                sys.stderr.write(
                    "pg-utf8-stream: invalid UTF-8 at input line %d byte %d "
                    "(context %s); refusing to load altered text. Re-run the "
                    "preflight scan, repair the rows, or pass the documented "
                    "repair option.\n" % (lineno, pos, line[max(0, pos - 16):pos + 16].hex()))
                return 3
            line = line.decode("utf-8", "goat_latin1_fallback").encode("utf-8")
        out.write(line)
    out.flush()
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("mode", choices=["scan", "filter"])
    ap.add_argument("--label", default="stdin", help="name reported in scan output")
    ap.add_argument("--repair", choices=["none", "latin1"], default="none")
    a = ap.parse_args()
    stream = sys.stdin.buffer
    if a.mode == "scan":
        return scan(stream, a.label)
    return filt(stream, sys.stdout.buffer, a.repair)


if __name__ == "__main__":
    sys.exit(main())
