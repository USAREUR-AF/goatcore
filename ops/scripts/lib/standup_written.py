#!/usr/bin/env python3
"""What a stand-up wrote into the tree it runs from, kept on the operator machine.

Some stand-up steps write generated files into the tree: phase 8 records the mesh
addresses the hosts got (group_vars, the mesh inventory, the certificate manifest),
and phases 4 and 12 regenerate a site's mesh zone. The source-of-record check
(source_of_record.py) refuses a tree whose files differ from the commits it was
assembled from, so the very next play of the same run was refused for the
stand-up's own write.

Each such step calls record() with the files it wrote. The check then treats a
changed file as the stand-up's own exactly when its bytes are the bytes recorded
here: a later hand edit changes the hash and is refused as before. The record
lives in the operator's state directory, never in the tree.
"""
import hashlib
import json
import os
import re
from pathlib import Path

NAME = "standup-written.json"


def state_dir(repo):
    """`deploy.state_dir` in ops/goatbuilder/goatbuilder.yaml, else ~/.goat/state
    (the same rule goatbuilder and standup-mesh-record.py follow)."""
    override = os.environ.get("GOAT_STANDUP_STATE_DIR")
    if override:
        return Path(override).expanduser()
    try:
        text = (Path(repo) / "ops/goatbuilder/goatbuilder.yaml").read_text()
    except OSError:
        return Path("~/.goat/state").expanduser()
    inside = False
    for line in text.splitlines():
        if re.match(r"deploy:", line):
            flow = re.search(r"\bstate_dir:[ \t]*([^,}\s]+)", line)
            if flow:
                return Path(flow.group(1).strip("\"'")).expanduser()
            inside = True
            continue
        if inside:
            if line.strip() and line[0] not in " \t":
                break
            key = re.match(r"[ \t]+state_dir:[ \t]*([^\s#]+)", line)
            if key:
                return Path(key.group(1).strip("\"'")).expanduser()
    return Path("~/.goat/state").expanduser()


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def recorded(repo):
    """{tree-relative path: sha256} of what the stand-up last wrote; {} when nothing is recorded."""
    try:
        doc = json.loads((state_dir(repo) / NAME).read_text())
    except (OSError, ValueError):
        return {}
    return doc if isinstance(doc, dict) else {}


def record(repo, paths):
    """Record the current bytes of each of `paths` (absolute, or relative to the tree)."""
    repo = Path(repo).resolve()
    doc = recorded(repo)
    for p in paths:
        p = Path(p)
        full = p if p.is_absolute() else repo / p
        try:
            rel = str(full.resolve().relative_to(repo))
        except ValueError:
            continue  # outside the tree: not a file the check judges
        doc[rel] = digest(full)
    target = state_dir(repo) / NAME
    target.parent.mkdir(parents=True, exist_ok=True)
    tmp = target.with_name(target.name + ".tmp")
    tmp.write_text(json.dumps(doc, indent=2, sort_keys=True) + "\n")
    os.replace(tmp, target)


def is_recorded(repo, rel):
    want = recorded(repo).get(rel)
    try:
        return bool(want) and digest(Path(repo) / rel) == want
    except OSError:
        return False
