#!/usr/bin/env python3
"""Record one goatbuilder release's configuration files for the A2 test
(crates/goatbuilder/tests/config_compat.rs; gate row A2).

  record.py --tag goatbuilder/vX.Y.Z --bin <that release's goatbuilder-linux-amd64> --sums <its SHA256SUMS>
  record.py --tag goatbuilder/vX.Y.Z --unpublished "<why>"

It reads, from the tag's own tree: every goatbuilder.yaml under
ops/goatbuilder/fixtures/, every refusal fixture under
crates/goatbuilder/tests/fixtures/configs/, and every whole example
(a ```yaml block holding `schema: goatbuilder/v1`) in the tag's docs/. Each is
validated by the release's own binary, and the file and its outcome are added to
configs/manifest.yaml: the set for that release. A file's content is stored once
(configs/files/, named by its SHA-256), however many releases shipped it.

The binary is taken only when its SHA-256 is the goatbuilder-linux-amd64 entry of
the SHA256SUMS published with the tag and it reports the tag's version.
`named:` (the catalog code a release's documents give a refusal) is added by hand.

Needs PyYAML, and git with the tag fetched.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

import yaml

HERE = os.path.dirname(os.path.abspath(__file__))
CONFIGS = os.path.join(HERE, "configs")
REPO = os.path.abspath(os.path.join(HERE, "..", "..", "..", ".."))
SCHEMA_LINE = re.compile(r'^schema:\s*"?goatbuilder/v1"?\s*$', re.M)
FENCE = re.compile(r"^```ya?ml\s*\n(.*?)^```", re.M | re.S)
TREES = ("ops/goatbuilder/fixtures/", "crates/goatbuilder/tests/fixtures/configs/")


def git(*args):
    return subprocess.run(["git", "-C", REPO, *args], capture_output=True, text=True, check=True).stdout


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def tag_configs(tag):
    """(source, text) for each configuration file of the tag's tree, and each whole example in its documents."""
    out = []
    for path in git("ls-tree", "-r", "--name-only", tag).splitlines():
        if path.endswith(".yaml") and path.startswith(TREES):
            text = git("show", f"{tag}:{path}")
            if path.startswith(TREES[1]) or SCHEMA_LINE.search(text):
                out.append((path, text))
        elif path.endswith(".md") and path.startswith("docs/"):
            text = git("show", f"{tag}:{path}")
            for i, m in enumerate(FENCE.finditer(text)):
                if SCHEMA_LINE.search(m.group(1)):
                    out.append((f"{path}#example-{i + 1}", m.group(1)))
    return out


def outcome(binary, path):
    home = tempfile.mkdtemp(prefix="config-compat-home-")
    try:
        env = dict(os.environ, HOME=home, GOAT_LANG="en")
        env.pop("GOAT_EXPLAIN", None)
        r = subprocess.run([binary, "-o", "json", "validate", "-C", REPO, path], capture_output=True, text=True, env=env)
    finally:
        shutil.rmtree(home, ignore_errors=True)
    try:
        doc = json.loads(r.stdout)
    except ValueError:
        doc = {}
    if r.returncode == 0:
        return "valid", None
    if r.returncode == 1 and doc.get("schema") == "goatbuilder.output/v1":
        for c in doc.get("checks") or []:
            if not c.get("ok"):
                m = re.match(r"^(GB-\d{3}): ", str(c.get("detail", "")))
                return "refused", (m.group(1) if m else None)
        return "refused", None
    return "error", doc.get("code")


def load():
    path = os.path.join(CONFIGS, "manifest.yaml")
    if not os.path.exists(path):
        return {"releases": [], "configs": []}, ""
    with open(path) as f:
        text = f.read()
    header = "".join(l for l in text.splitlines(True) if l.startswith("#"))
    return yaml.safe_load(text) or {"releases": [], "configs": []}, header


def save(m, header):
    m["releases"].sort(key=lambda r: [int(x) for x in r["tag"].split("/v", 1)[1].split(".")])
    with open(os.path.join(CONFIGS, "manifest.yaml"), "w") as f:
        f.write(header)
        yaml.safe_dump(m, f, sort_keys=False, allow_unicode=True, width=1000)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--tag", required=True)
    ap.add_argument("--bin")
    ap.add_argument("--sums")
    ap.add_argument("--unpublished", help="the tag was never published; why")
    a = ap.parse_args()
    m, header = load()
    if any(r["tag"] == a.tag for r in m["releases"]):
        print(f"record.py: {a.tag} is already recorded", file=sys.stderr)
        return 2
    if a.unpublished:
        m["releases"].append({"tag": a.tag, "binary_sha256": "", "note": a.unpublished})
        save(m, header)
        return 0
    if not (a.bin and a.sums):
        print("record.py: give --bin and --sums, or --unpublished", file=sys.stderr)
        return 2
    with open(a.bin, "rb") as f:
        digest = sha256(f.read())
    sums = {}
    with open(a.sums) as f:
        for line in f:
            parts = line.split()
            if len(parts) == 2:
                sums[parts[1].lstrip("*")] = parts[0].lower()
    if sums.get("goatbuilder-linux-amd64") != digest:
        print(f"record.py: {a.bin} is not the goatbuilder-linux-amd64 that {a.sums} lists", file=sys.stderr)
        return 2
    version = subprocess.run([a.bin, "--version"], capture_output=True, text=True).stdout.split()[-1:]
    if version != [a.tag.split("/v", 1)[1]]:
        print(f"record.py: {a.bin} reports {version}, not {a.tag}", file=sys.stderr)
        return 2
    files = os.path.join(CONFIGS, "files")
    os.makedirs(files, exist_ok=True)
    for source, text in tag_configs(a.tag):
        data = text.encode()
        stem = os.path.splitext(os.path.basename(source.split("#")[0]))[0]
        name = f"{sha256(data)[:16]}-{stem}.yaml"
        dest = os.path.join(files, name)
        if not os.path.exists(dest):
            with open(dest, "wb") as f:
                f.write(data)
        got, code = outcome(a.bin, dest)
        entry = {"release": a.tag, "source": source, "file": "files/" + name, "expect": got}
        if got != "valid":
            entry["code"] = code
        m["configs"].append(entry)
        print(f"record.py: {a.tag} {source}: {got}" + (f" ({code})" if code else ""))
    m["releases"].append({"tag": a.tag, "binary_sha256": digest, "note": ""})
    save(m, header)
    return 0


if __name__ == "__main__":
    sys.exit(main())
