#!/usr/bin/env bash
# parity-deploy.sh — the parity gate for `goatbuilder deploy` against
# ops/scripts/deploy-component.sh (goatbuilder-conformance.md §The parity gate).
#
# Both are run with --dry-run for every component the pins declare on each
# goatnet named, and the decisions that determine what a real run would do are
# compared: the member groups and the transport, the play and its target flag,
# the ledger component, per group the declared ref and the digest the verify
# step would require, and the secrets each side says the play needs and are
# not set here (role-default bindings included: A|B|C means any one of them),
# for a release component what each side fetched and verified, or why it
# refuses, and what each side says a real run would refuse before any host
# (fresh source, a pin with no digest, an intent that does not verify, a release
# that does not verify, unset secrets). Any difference fails. No host is touched.
#
#
# Off the mesh, run it as CI does: `GOAT_MESH_PROBE=0 parity-deploy.sh …`. The
# script avenue probes each goatnet's mesh host on tcp/22 to decide the
# transport, and a laptop that is not on the mesh waits for that probe on every
# component rather than failing, so the gate looks hung.
#
# usage: parity-deploy.sh <path-to-goatbuilder-binary> [goatnet ...]
#        (default goatnets: every live rung in ops/release/ladder.yaml)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
BIN="${1:?usage: parity-deploy.sh <goatbuilder binary> [goatnet ...]}"; shift || true
GOATNETS=("$@")
if [ ${#GOATNETS[@]} -eq 0 ]; then
  while IFS= read -r g; do [ -n "$g" ] && GOATNETS+=("$g"); done < <(python3 - "$REPO/ops/release/ladder.yaml" <<'PY'
import sys, yaml
for r in (yaml.safe_load(open(sys.argv[1])) or {}).get("rungs") or []:
    if r.get("status") == "live": print(r["goatnet"])
PY
)
fi
# A dry run runs the fresh-source check; pin its trunk to this checkout's commit,
# so no pair fetches. parity-fresh-source.sh holds that check itself.
export GOAT_FRESH_SOURCE_TRUNK="${GOAT_FRESH_SOURCE_TRUNK:-$(git -C "$REPO" rev-parse HEAD)}"
COMPONENTS=()
while IFS= read -r c; do [ -n "$c" ] && COMPONENTS+=("$c"); done < <(python3 - "$REPO/ops/pins/fleet-pins.yaml" <<'PY'
import sys, yaml
print("\n".join(sorted((yaml.safe_load(open(sys.argv[1])) or {}).get("components") or {})))
PY
)

# The facts, in a form both sides can be reduced to.
script_facts() {   # <component> <goatnet>
  "$REPO/ops/scripts/deploy-component.sh" "$1" --goatnet "$2" --dry-run 2>&1 | python3 -c '
import re, sys
for line in sys.stdin:
    line = line.strip()
    if line.startswith("deploy-component: "): print("refused", line[len("deploy-component: "):]); continue
    m = re.match(r"goatnet \S+ \(\S+\): groups \[(.*)\] via (\w+)", line)
    if m: print("groups", m.group(1)); print("transport", m.group(2)); continue
    m = re.match(r"playbook (\S+) (.*?)<group>; ledger component (\S+)", line)
    if m: print("play", m.group(1)); print("flag", m.group(2).strip()); print("ledger", m.group(3)); continue
    m = re.match(r"(\S+): deploying (\S+)", line)
    if m: print("ref", m.group(1), m.group(2)); continue
    m = re.match(r"DRY: would read \S+ on (\S+) and require ref digest (\S+)", line)
    if m: print("digest", m.group(1), m.group(2))
    m2 = re.search(r"DRY: would read \S+ on (\S+) .* and served payloads (\{.*\})$", line)
    if m2: print("served", m2.group(1), m2.group(2))
    if m or m2: continue
    m = re.search(r"templates (\d+) variable\(s\) from your environment; NOT SET: (.*)$", line)
    if m: print("secrets", m.group(1), " ".join(sorted(m.group(2).split()))); continue
    m = re.match(r"skipping \(absent: true, topology not skew\): (.*)", line)
    if m: print("skipped", " ".join(sorted(m.group(1).split()))); continue
    m = re.match(r"(\S+): release verified: (.*)", line)
    if m: print("release", m.group(1), m.group(2)); continue
    m = re.match(r"(\S+): (GB-381 .*)", line)
    if m: print("would-refuse release", m.group(1), m.group(2)); continue
    if line.startswith("DRY: would read the file "): print("file", line[len("DRY: "):]); continue
    m = re.search(r"DRY: a real run would refuse before any host \((.*)\)", line)
    if m: print("verdict-refused", m.group(1)); continue
    m = re.match(r"(\S+): no \S+ in \S+ — nothing declared, skipping", line)
    if m: print("undeclared", m.group(1)); continue
    m = re.match(r"(ladder|fresh-source|secrets): REFUSE — a real run stops here", line)
    if m: print("would-refuse", m.group(1)); continue
    m = re.match(r"GB-383 REFUSE — (.*)", line)
    if m: print("would-refuse intent", m.group(1)); continue
    m = re.match(r"(\S+): GB-382 REFUSE", line)
    if m: print("would-refuse digest", m.group(1)); continue
' | sort
}
binary_facts() {   # <component> <goatnet>
  "$BIN" -C "$REPO" -o json deploy --env "$2" --component "$1" --dry-run 2>&1 | python3 -c '
import json, re, sys
raw = sys.stdin.read()
try: r = json.loads(raw)
except Exception:
    for line in raw.splitlines():
        if line.startswith("goatbuilder: "): print("refused", line[len("goatbuilder: "):])
    sys.exit()
for c in r["checks"]:
    n, d = c["name"], c["detail"]
    if n.endswith("/transport"):
        m = re.match(r"groups \[(.*)\] via (\w+)", d); print("groups", m.group(1)); print("transport", m.group(2))
    elif n.endswith("/playbook"):
        m = re.match(r"(\S+) (.*?)<group>; ledger component (\S+)", d); print("play", m.group(1)); print("flag", m.group(2).strip()); print("ledger", m.group(3))
    elif n.endswith("/deploy"):
        g = n.split("/")[1]; m = re.match(r"DRY: deploying (\S+):", d); print("ref", g, m.group(1))
    elif n.endswith("/verify"):
        g = n.split("/")[1]; m = re.search(r"require ref digest (\S+)", d); print("digest", g, m.group(1))
        m2 = re.search(r" and served payloads (\{.*\})$", d)
        if m2: print("served", g, m2.group(1))
    elif n.endswith("/secrets"):
        m = re.search(r"templates (\d+) variable\(s\) from your environment; NOT SET: (.*?)(;|$)", d)
        if m: print("secrets", m.group(1), " ".join(sorted(m.group(2).split())))
    elif n.endswith("/skipped"):
        m = re.search(r"skew: (.*)", d); print("skipped", " ".join(sorted(m.group(1).split())))
    elif n.endswith("/release"):
        g = n.split("/")[1]
        print("release", g, d[len("release verified: "):]) if c["ok"] else print("would-refuse release", g, d)
    elif n.endswith("/file"): print("file", d[len("DRY: "):])
    elif "nothing declared, skipping" in d:
        print("undeclared", n.split("/")[1])
    if not c["ok"]:
        if n == "fresh-source": print("would-refuse fresh-source")
        elif n.endswith("/ladder"): print("would-refuse ladder")
        elif n.endswith("/intent"): print("would-refuse intent", d.split("REFUSE — ", 1)[-1])
        elif n.endswith("/secrets"): print("would-refuse secrets")
        elif n.endswith("/digest"): print("would-refuse digest", n.split("/")[1])
        else:
            m = re.search(r"DRY: a real run would refuse before any host \((.*)\)", d)
            if m: print("verdict-refused", m.group(1))
' | sort
}

# Nothing to compare is a failure, never a pass: a python without PyYAML read
# no components and the gate reported "parity ok: 0 pairs".
[ ${#COMPONENTS[@]} -gt 0 ] && [ ${#GOATNETS[@]} -gt 0 ] || { echo "parity FAIL: no components or goatnets read (is PyYAML importable by python3?)"; exit 1; }
fail=0; n=0
for g in "${GOATNETS[@]}"; do
  for c in "${COMPONENTS[@]}"; do
    n=$((n+1))
    a="$(script_facts "$c" "$g" || true)"; b="$(binary_facts "$c" "$g" || true)"
    # A refusal decides the run: what each side printed before it (the script
    # notes the skipped groups before its release-ladder gate refuses, GB-373)
    # is not compared, only the refusal itself.
    if printf '%s\n' "$a" | grep -q '^refused '; then a="$(printf '%s\n' "$a" | grep '^refused ')"; fi
    if printf '%s\n' "$b" | grep -q '^refused '; then b="$(printf '%s\n' "$b" | grep '^refused ')"; fi
    if [ "$a" != "$b" ]; then
      fail=$((fail+1)); echo "PARITY FAIL: $c on $g"
      { diff <(printf '%s\n' "$a") <(printf '%s\n' "$b") || true; } | sed 's/^/    /' | head -20
    fi
  done
done
if [ "$fail" = 0 ]; then echo "parity ok: $n component/goatnet pairs decide identically (script vs goatbuilder deploy --dry-run)"; else echo "parity FAIL: $fail of $n pairs differ"; exit 1; fi
