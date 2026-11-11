#!/usr/bin/env bash
# federation-e2e-key.sh TREE SCENARIO: the input key of one scenario of the two-sovereign job
# (.github/workflows/federation-e2e.yml), rule 3 of docs/design/ci-results-follow-inputs.md.
#
# The key is a sha256 over, in this order, each part named and each file's path and content:
#   - the scenario's name;
#   - the workflow file itself;
#   - every file under the workflow's pull_request `paths:` list;
#   - every script the workflow's `run:` steps call (an ops/... path on a run line);
#   - crates/Cargo.lock;
#   - the Rust toolchain the workflow names;
#   - ops/federation-e2e/host/Dockerfile and the files beside it (the host image's inputs,
#     its base named by digest) and ops/federation-e2e/requirements.lock.
# Files are read from TREE's working copy, so an uncommitted change counts. A path outside all
# of these does not change the key; that is what lets a pass be reused, and why the
# declaration is wide (whole directories, the workflow and its scripts).
set -euo pipefail
TREE="$(cd "${1:?usage: federation-e2e-key.sh TREE SCENARIO}" && pwd)"; SCEN="${2:?scenario}"
python3 - "$TREE" "$SCEN" <<'PY'
import hashlib, os, re, subprocess, sys, yaml
tree, scen = sys.argv[1], sys.argv[2]
wf_rel = ".github/workflows/federation-e2e.yml"
wf = yaml.safe_load(open(os.path.join(tree, wf_rel)))
on = wf.get("on", wf.get(True, {}))  # YAML 1.1 reads `on:` as True
paths = (on.get("pull_request") or {}).get("paths") or []
def ls(spec):
    out = subprocess.run(["git", "-C", tree, "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", spec],
                         check=True, capture_output=True).stdout.decode()
    return [p for p in out.split("\0") if p]
files = set()
for p in paths:
    files.update(ls(f":(glob){p}"))
runs = "\n".join(st.get("run", "") for j in wf["jobs"].values() for st in j.get("steps", []))
files.update(f for f in re.findall(r"\bops/[A-Za-z0-9_./-]+", runs) if os.path.isfile(os.path.join(tree, f)))
files.update(ls(":(glob)ops/federation-e2e/host/**"))
files.update(["crates/Cargo.lock", "ops/federation-e2e/requirements.lock", wf_rel])
toolchain = re.findall(r"toolchain:\s*'?([0-9][0-9.]*)'?", open(os.path.join(tree, wf_rel)).read())
h = hashlib.sha256()
def part(name, data):
    h.update(name.encode() + b"\0" + hashlib.sha256(data).hexdigest().encode() + b"\n")
part("scenario", scen.encode())
part("toolchain", " ".join(toolchain).encode())
for f in sorted(files):
    full = os.path.join(tree, f)
    part(f, open(full, "rb").read() if os.path.isfile(full) else b"<absent>")
print(h.hexdigest())
PY
