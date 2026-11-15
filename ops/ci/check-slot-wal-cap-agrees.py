#!/usr/bin/env python3
"""check-slot-wal-cap-agrees: one statement of Postgres's slot WAL cap.

`max_slot_wal_keep_size` is set in two places, and both are meant:

  * the Spock image's postgresql.conf (ops/postgres/spock/conf/postgresql.conf),
    which every site's postgres-a starts with;
  * the postgres-streaming-replication role (postgres_max_slot_wal_keep_size),
    which raises it with ALTER SYSTEM on the primary of a site with a
    streaming pair (intra_site_pair: true). postgresql.auto.conf is read after
    the configuration file, so on that primary the role's value is the one
    in effect.

The rule and both values are written once, in ops/postgres/spock/README.md,
"The slot WAL cap". This check refuses:

  * a README section whose two values are not the two configured values;
  * a size written next to max_slot_wal_keep_size in a file that should
    point at the README instead (the runbook, the alert rules, the exporter
    queries), where a value written down goes stale when either source moves.

  check-slot-wal-cap-agrees.py              check this repository; exit 0 or 1
  check-slot-wal-cap-agrees.py --self-test  plant each violation on a copy and
                                            require a refusal, then check
  --repo DIR                                the repository root
"""
from __future__ import annotations

import argparse
import os
import re
import shutil
import sys
import tempfile

IMAGE_CONF = "ops/postgres/spock/conf/postgresql.conf"
ROLE_DEFAULTS = "ops/ansible/roles/postgres-streaming-replication/defaults/main.yml"
README = "ops/postgres/spock/README.md"
SECTION = "## The slot WAL cap"
# Files that name the cap and must not carry a size of their own.
POINTERS = [
    "docs/operations/post-partition-convergence-runbook.md",
    "ops/compose/mgmt-host/monitoring/rules/spock.yml",
    "ops/compose/mgmt-host/monitoring/rules/postgres-storage.yml",
    "ops/compose/mgmt-host/monitoring/postgres-exporter-queries.yml",
]
SIZE = re.compile(r"\b(\d+)\s?(GiB|GB|MiB|MB)\b")


def norm(v: str) -> str:
    m = SIZE.search(v)
    return f"{m.group(1)}{m.group(2).replace('i', '')}" if m else v.strip()


def read(repo: str, rel: str) -> str:
    with open(os.path.join(repo, rel), encoding="utf-8") as fh:
        return fh.read()


def check(repo: str) -> list[str]:
    bad = []
    m = re.search(r"^max_slot_wal_keep_size\s*=\s*(\S+)", read(repo, IMAGE_CONF), re.M)
    image = norm(m.group(1)) if m else None
    m = re.search(r'^postgres_max_slot_wal_keep_size:\s*"?([^"\s]+)"?', read(repo, ROLE_DEFAULTS), re.M)
    role = norm(m.group(1)) if m else None
    if image is None:
        bad.append(f"{IMAGE_CONF}: no max_slot_wal_keep_size line")
    if role is None:
        bad.append(f"{ROLE_DEFAULTS}: no postgres_max_slot_wal_keep_size default")
    text = read(repo, README)
    start = text.find(SECTION + "\n")
    if start < 0:
        bad.append(f"{README}: no section {SECTION!r} stating the rule and both values")
    else:
        rest = text[start + len(SECTION) + 1:]
        nxt = rest.find("\n## ")
        body = rest if nxt < 0 else rest[:nxt]
        said = {norm(f"{a}{b}") for a, b in SIZE.findall(body)}
        for name, v in (("the image's", image), ("the role's", role)):
            if v and v not in said:
                bad.append(f"{README} {SECTION!r}: does not state {name} value {v}")
        for v in sorted(said - {image, role}):
            bad.append(f"{README} {SECTION!r}: states {v}, which neither the image nor the role sets")
    for rel in POINTERS:
        for n, line in enumerate(read(repo, rel).splitlines(), 1):
            if "max_slot_wal_keep_size" in line and SIZE.search(line):
                bad.append(f"{rel}:{n}: a size next to max_slot_wal_keep_size; point at {README}, {SECTION!r}")
    return bad


def self_test(repo: str) -> int:
    def planted(edit) -> list[str]:
        with tempfile.TemporaryDirectory() as d:
            for rel in [IMAGE_CONF, ROLE_DEFAULTS, README, *POINTERS]:
                os.makedirs(os.path.dirname(os.path.join(d, rel)), exist_ok=True)
                shutil.copy(os.path.join(repo, rel), os.path.join(d, rel))
            edit(d)
            return check(d)

    def sub(rel, old, new):
        def edit(d):
            p = os.path.join(d, rel)
            s = open(p, encoding="utf-8").read()
            assert old in s, (rel, old)
            open(p, "w", encoding="utf-8").write(s.replace(old, new, 1))
        return edit

    m = re.search(r"^max_slot_wal_keep_size\s*=\s*\S+", read(repo, IMAGE_CONF), re.M)
    cases = {
        "the image's value moves": sub(IMAGE_CONF, m.group(0), "max_slot_wal_keep_size = 3GB") if m else None,
        "the runbook writes a size": sub(POINTERS[0], "\n", "\n- `retained_wal` < `max_slot_wal_keep_size` (default 1 GiB).\n"),
        "the section is missing": sub(README, SECTION, "## Something else"),
    }
    failed = 0
    for name, edit in cases.items():
        if edit is None or not planted(edit):
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
    print("check-slot-wal-cap-agrees: ok, the README states the image's and the role's cap, and no pointer file writes its own")
    return 0


if __name__ == "__main__":
    sys.exit(main())
