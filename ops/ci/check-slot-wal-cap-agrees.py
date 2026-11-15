#!/usr/bin/env python3
"""check-slot-wal-cap-agrees: one statement of Postgres's slot WAL cap.

`max_slot_wal_keep_size` is set in two places, and both are meant:

  * the Spock image's postgresql.conf (ops/postgres/spock/conf/postgresql.conf),
    which every site's postgres-a starts with. Postgres applies the LAST
    assignment it reads, following `include`, `include_if_exists` and
    `include_dir` in order, so this check does the same;
  * the postgres-streaming-replication role (postgres_max_slot_wal_keep_size),
    which raises it with ALTER SYSTEM on the primary of a site with a
    streaming pair (intra_site_pair: true).

The rule and both values are written once, in ops/postgres/spock/README.md,
"The slot WAL cap", in a table with one row per place. This check refuses:

  * a table row whose value is not the value its place sets (the row is found
    by its label: the image's names conf/postgresql.conf, the role's names
    postgres_max_slot_wal_keep_size), or a missing row;
  * a command line that passes the setting (`-c max_slot_wal_keep_size=`),
    which the README says nothing does;
  * a size or a byte count written on the same line as max_slot_wal_keep_size,
    or on the line after it, in any tracked file other than the two places
    and the README's section: such a value goes stale when either place moves.
    The unlimited sentinel 2^63-1 is not a size. A file that sets a value on
    purpose (a drill) is listed in ops/ci/slot-wal-cap-mentions.allow with
    its reason.

Sizes compare in bytes, with Postgres's units (kB, MB, GB, TB, binary; a bare
number is megabytes, the setting's unit), so 1024MB and 1GB agree and prose
in the section may mention other sizes.

  check-slot-wal-cap-agrees.py              check this repository; exit 0 or 1
  check-slot-wal-cap-agrees.py --self-test  plant each violation on a copy and
                                            require a refusal, then check
  --repo DIR                                the repository root
"""
from __future__ import annotations

import argparse
import glob
import os
import re
import shutil
import subprocess
import sys
import tempfile

IMAGE_CONF = "ops/postgres/spock/conf/postgresql.conf"
ROLE_DEFAULTS = "ops/ansible/roles/postgres-streaming-replication/defaults/main.yml"
README = "ops/postgres/spock/README.md"
SECTION = "## The slot WAL cap"
ALLOW = "ops/ci/slot-wal-cap-mentions.allow"
SELF = ("ops/ci/check-slot-wal-cap-agrees.py", "ops/ci/test_check_slot_wal_cap_agrees.py", ALLOW)
SETTING = "max_slot_wal_keep_size"
SENTINEL = "9223372036854775807"

UNITS = {"kb": 1024, "kib": 1024, "mb": 1024 ** 2, "mib": 1024 ** 2,
         "gb": 1024 ** 3, "gib": 1024 ** 3, "tb": 1024 ** 4, "tib": 1024 ** 4}
SIZE = re.compile(r"(?<![\w.])(\d+(?:\.\d+)?)\s?(TiB|TB|GiB|GB|MiB|MB|KiB|kB|KB)\b")
BYTES = re.compile(r"(?<![\w.])(\d{7,})(?![\w.])")
ASSIGN = re.compile(r"^\s*" + SETTING + r"\s*=?\s*'?([^'#\s]+)'?", re.M)
INCLUDE = re.compile(r"^\s*(include|include_if_exists|include_dir)\s*=?\s*'([^']+)'", re.M)
COMMAND_LINE = re.compile(r"(-c\s*|--)" + SETTING + r"\s*=")


def size_bytes(v: str):
    """Bytes in a Postgres size (`1GB`, `1.5 GB`, `512kB`, a bare number of
    megabytes), -1 for unlimited, or None when it is not a size."""
    v = v.strip().strip("'\"")
    if v == "-1":
        return -1
    m = re.fullmatch(r"(\d+(?:\.\d+)?)\s?([A-Za-z]*)", v)
    if not m:
        return None
    unit = m.group(2).lower() or "mb"
    if unit not in UNITS:
        return None
    return int(float(m.group(1)) * UNITS[unit])


def human(b) -> str:
    if b is None:
        return "nothing readable"
    if b == -1:
        return "-1 (unlimited)"
    for name, n in (("TB", 1024 ** 4), ("GB", 1024 ** 3), ("MB", 1024 ** 2), ("kB", 1024)):
        if b >= n and b % n == 0:
            return f"{b // n}{name}"
    return f"{b} bytes"


def read(repo: str, rel: str) -> str:
    with open(os.path.join(repo, rel), encoding="utf-8") as fh:
        return fh.read()


def image_value(repo: str):
    """The cap the image's configuration ends with: the last assignment, read
    in the order Postgres reads, includes followed. (bytes, where) or (None, None)."""
    found = [None, None]

    def walk(path: str, depth: int):
        if depth > 10 or not os.path.isfile(path):
            return
        text = open(path, encoding="utf-8").read()
        for n, line in enumerate(text.splitlines(), 1):
            inc = INCLUDE.match(line)
            if inc:
                target = os.path.join(os.path.dirname(path), inc.group(2))
                if inc.group(1) == "include_dir":
                    for f in sorted(glob.glob(os.path.join(target, "*.conf"))):
                        walk(f, depth + 1)
                else:
                    walk(target, depth + 1)
                continue
            m = ASSIGN.match(line)
            if m:
                found[0] = size_bytes(m.group(1))
                found[1] = f"{os.path.relpath(path, repo)}:{n}"

    walk(os.path.join(repo, IMAGE_CONF), 0)
    return found[0], found[1]


def role_value(repo: str):
    m = re.search(r'^postgres_max_slot_wal_keep_size:\s*"?([^"\s]+)"?', read(repo, ROLE_DEFAULTS), re.M)
    return size_bytes(m.group(1)) if m else None


def section(text: str):
    """(first line number, body) of the README's section, or (None, None)."""
    start = text.find(SECTION + "\n")
    if start < 0:
        return None, None
    rest = text[start + len(SECTION) + 1:]
    nxt = rest.find("\n## ")
    body = rest if nxt < 0 else rest[:nxt + 1]
    return text[:start].count("\n") + 1, body


def table_values(body: str):
    """{'image': value cell, 'role': value cell} from the section's table, by label."""
    out = {}
    value_col = None
    for line in body.splitlines():
        if not line.strip().startswith("|"):
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if value_col is None:
            if "Value" in cells:
                value_col = cells.index("Value")
            continue
        if set("".join(cells)) <= set("-: "):
            continue
        where = cells[0]
        val = cells[value_col] if value_col < len(cells) else ""
        if "conf/postgresql.conf" in where:
            out["image"] = val
        elif "postgres_max_slot_wal_keep_size" in where:
            out["role"] = val
    return out


def tracked_files(repo: str) -> list[str]:
    r = subprocess.run(["git", "-C", repo, "ls-files"], capture_output=True, text=True)
    if r.returncode == 0 and r.stdout.strip():
        return [f for f in r.stdout.splitlines() if os.path.isfile(os.path.join(repo, f))]
    out = []
    for root, dirs, files in os.walk(repo):
        dirs[:] = [d for d in dirs if d not in (".git", "target", "node_modules")]
        out += [os.path.relpath(os.path.join(root, f), repo) for f in files]
    return out


def allowed(repo: str, bad: list[str]) -> set[str]:
    p = os.path.join(repo, ALLOW)
    if not os.path.isfile(p):
        return set()
    out = set()
    for n, line in enumerate(open(p, encoding="utf-8").read().splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        parts = line.split(None, 1)
        if len(parts) < 2 or not parts[1].strip():
            bad.append(f"{ALLOW}:{n}: '{parts[0]}' needs its reason after the path")
            continue
        out.add(parts[0])
    return out


def has_value(line: str) -> bool:
    if SIZE.search(line):
        return True
    return any(m.group(1) != SENTINEL for m in BYTES.finditer(line))


def check(repo: str) -> list[str]:
    bad: list[str] = []
    image, image_at = image_value(repo)
    role = role_value(repo)
    if image_at is None:
        bad.append(f"{IMAGE_CONF}: no {SETTING} line (includes followed)")
    if role is None:
        bad.append(f"{ROLE_DEFAULTS}: no postgres_max_slot_wal_keep_size default")

    text = read(repo, README)
    sec_line, body = section(text)
    if body is None:
        bad.append(f"{README}: no section {SECTION!r} stating the rule and both values")
        body = ""
    else:
        rows = table_values(body)
        for key, name, want in (("image", "the image's", image), ("role", "the role's", role)):
            if key not in rows:
                bad.append(f"{README} {SECTION!r}: no table row for {name} value (a row naming "
                           f"{'conf/postgresql.conf' if key == 'image' else 'postgres_max_slot_wal_keep_size'})")
                continue
            got = size_bytes(rows[key])
            if want is not None and got != want:
                where = f" ({image_at})" if key == "image" and image_at else ""
                bad.append(f"{README} {SECTION!r}: the row for {name} value says {rows[key]!r}, "
                           f"but {name.split()[0]} {name.split()[1]} value is {human(want)}{where}")

    allow = allowed(repo, bad)
    sec_lines = set()
    if sec_line is not None:
        sec_lines = set(range(sec_line, sec_line + body.count("\n") + 1))
    for rel in tracked_files(repo):
        if rel in SELF or rel in allow:
            continue
        try:
            lines = read(repo, rel).splitlines()
        except (UnicodeDecodeError, OSError):
            continue
        for i, line in enumerate(lines):
            if SETTING not in line:
                continue
            n = i + 1
            if COMMAND_LINE.search(line) and not rel.endswith(".md"):
                bad.append(f"{rel}:{n}: a command line sets {SETTING}; the cap is set only by "
                           f"{IMAGE_CONF} and the role ({README}, {SECTION!r})")
                continue
            if rel == IMAGE_CONF and ASSIGN.match(line):
                continue
            if rel == ROLE_DEFAULTS and line.startswith("postgres_max_slot_wal_keep_size:"):
                continue
            if rel == README and n in sec_lines:
                continue
            window = [line] + ([lines[i + 1]] if i + 1 < len(lines) and SETTING not in lines[i + 1] else [])
            if any(has_value(w) for w in window):
                bad.append(f"{rel}:{n}: a size beside {SETTING}; point at {README}, {SECTION!r}, "
                           f"or list the file in {ALLOW} with its reason")
    return bad


def self_test(repo: str) -> int:
    def planted(edit) -> list[str]:
        with tempfile.TemporaryDirectory() as d:
            for rel in [IMAGE_CONF, ROLE_DEFAULTS, README]:
                os.makedirs(os.path.dirname(os.path.join(d, rel)), exist_ok=True)
                shutil.copy(os.path.join(repo, rel), os.path.join(d, rel))
            edit(d)
            return check(d)

    def append(rel, text):
        def edit(d):
            p = os.path.join(d, rel)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "a", encoding="utf-8") as fh:
                fh.write(text)
        return edit

    def swap(d):
        p = os.path.join(d, README)
        s = open(p, encoding="utf-8").read()
        rows = table_values(section(s)[1])
        s = s.replace(f"| {rows['image']} |", "| @IMAGE@ |", 1).replace(f"| {rows['role']} |", f"| {rows['image']} |", 1)
        open(p, "w", encoding="utf-8").write(s.replace("| @IMAGE@ |", f"| {rows['role']} |", 1))

    cases = {
        "a later image line moves the cap": append(IMAGE_CONF, f"\n{SETTING} = 3GB\n"),
        "the role's value moves": append(ROLE_DEFAULTS, ""),  # replaced below
        "the README swaps the two values": swap,
        "a command line passes the setting": append("ops/compose/x/docker-compose.yml", f"command: postgres -c {SETTING}=4GB\n"),
        "a file writes a size beside the setting": append("docs/x.md", f"the {SETTING}\n(default 1 GiB)\n"),
    }

    def role_moves(d):
        p = os.path.join(d, ROLE_DEFAULTS)
        s = open(p, encoding="utf-8").read()
        s = re.sub(r'^postgres_max_slot_wal_keep_size:.*$', 'postgres_max_slot_wal_keep_size: "8GB"', s, flags=re.M)
        open(p, "w", encoding="utf-8").write(s)
    cases["the role's value moves"] = role_moves

    failed = 0
    for name, edit in cases.items():
        if not planted(edit):
            print(f"self-test: NOT refused: {name}")
            failed += 1
    if failed:
        return 1
    print(f"self-test: each of {len(cases)} planted violations is refused")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--repo", default=os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..")))
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test and self_test(a.repo):
        return 1
    bad = check(a.repo)
    for b in bad:
        print(f"check-slot-wal-cap-agrees: {b}")
    if bad:
        return 1
    print("check-slot-wal-cap-agrees: ok, the README's table states the image's and the role's cap, "
          "and no other file writes its own")
    return 0


if __name__ == "__main__":
    sys.exit(main())
