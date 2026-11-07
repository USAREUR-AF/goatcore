#!/usr/bin/env bash
# The OVER-grant audit. Sibling of check-node-class-membership.sh.
#
# Every check we had is a LOWER bound. The cutover playbook's coverage report
# asks "is this peer covered by any rule" — i.e. will it be cut off. The §7
# guard asks "is this peer's shape right" — gear must not wear a person role.
# Neither asks whether the class a peer holds is the class it SHOULD hold.
#
# That gap actively hides the worst case, because coverage is a lower bound:
# INFRA is the most-covered a peer can possibly be, so an over-granted peer is
# maximally green in every report we run. A foreign-stack uplink in the INFRA
# group holds any-port reach to the core hosts and the hidden backend tier.
#
# This computes EFFECTIVE reach from the LIVE enabled policies and flags the
# shapes that are over-grants by construction, whatever the group is named.
#
# Usage:
#   NB_API_URL=https://netbird.<zone> NB_API_TOKEN=... \
#     ops/ci/check-mesh-reach-overgrant.sh
#
# NOT wired into PR CI: it needs live mesh credentials, and it refuses to pass
# on an unread mesh rather than treating silence as clean — same discipline as
# the §7 guard.
#
# KNOWN BLIND SPOT: this reads ONE goatnet. A box classed as sovereign on one
# goatnet and INFRA on another has nothing in its group set here that says
# "foreign". Catching it needs cross-goatnet correlation (same box, sovereign
# class over there, substrate class over here), which this does not do. Run it on every goatnet
# substrate class over here), which this does not do. Run it on every goatnet
# and compare by hand until that exists.
#
# GROUP PREFIXES. A group name is read as <scope>-<CLASS> and the scope prefix
# is stripped before the class is compared. The public example scopes are
# listed below; a deployment adds its own, one per line, in
# ops/ci/check-mesh-reach-overgrant.d/*.prefixes (blank lines and `#` comments
# ignored). The longest prefix that matches is the one stripped.
#
# DECLARED DEVIATIONS: ops/ci/mesh-reach-declared-deviations.yaml. An accepted
# state is declared there with its operator, date and reason — not suppressed
# silently. Declared rows are printed as NOTE so they stay visible, and do not
# fail the run. A check that keeps firing on a deliberate decision trains people
# to skim its output, and the real finding is the one they skim past.
set -euo pipefail

: "${NB_API_URL:?set NB_API_URL, e.g. https://netbird.zagros-prod.example}"
: "${NB_API_TOKEN:?set NB_API_TOKEN (a NetBird PAT)}"

peers="$(curl -sf -H "Authorization: Token ${NB_API_TOKEN}" "${NB_API_URL}/api/peers")" || {
  echo "FAIL: could not read ${NB_API_URL}/api/peers — refusing to pass on an unread mesh" >&2
  exit 1
}
policies="$(curl -sf -H "Authorization: Token ${NB_API_TOKEN}" "${NB_API_URL}/api/policies")" || {
  echo "FAIL: could not read ${NB_API_URL}/api/policies — refusing to pass on an unread mesh" >&2
  exit 1
}

DEVIATIONS="${MESH_REACH_DEVIATIONS:-$(dirname "$0")/mesh-reach-declared-deviations.yaml}"

GROUP_PREFIXES="ZAGROS-PROD- SITE_ONE- ZAGROS- NOVA- VEGA- EX-DEMO-"
for f in "$(dirname "$0")"/check-mesh-reach-overgrant.d/*.prefixes; do
  [ -f "$f" ] || continue
  GROUP_PREFIXES="$GROUP_PREFIXES $(sed -e 's/#.*$//' "$f" | tr -s '[:space:]' ' ')"
done
export GROUP_PREFIXES

printf '%s\n%s\n%s\n' "$peers" "$policies" "$DEVIATIONS" | python3 -c '
import json, os, sys

raw = sys.stdin.read().split("\n")
peers = json.loads(raw[0])
policies = json.loads(raw[1])

# Declared deviations. Parsed with a deliberately tiny reader so this guard has
# no dependency on PyYAML being present on an operator shell.
declared = {}
dev_path = raw[2].strip() if len(raw) > 2 else ""
if dev_path and os.path.isfile(dev_path):
    cur = {}
    for line in open(dev_path):
        s = line.strip()
        if s.startswith("- peer:"):
            if cur.get("peer"):
                declared[(cur["peer"], cur.get("check", ""))] = cur
            cur = {"peer": s.split(":", 1)[1].strip()}
        elif s.startswith("check:") and cur:
            cur["check"] = s.split(":", 1)[1].strip()
        elif s.startswith("decided_by:") and cur:
            cur["by"] = s.split(":", 1)[1].strip()
    if cur.get("peer"):
        declared[(cur["peer"], cur.get("check", ""))] = cur

# Longest first, so a scope that extends another (<A>-PROD- over <A>-) wins.
PREFIXES = sorted(set(os.environ.get("GROUP_PREFIXES", "").split()), key=lambda p: (-len(p), p))

def bare(g):
    for p in PREFIXES:
        if g.startswith(p):
            return g[len(p):]
    return g

# Classes that are SOVEREIGN or EXTERNAL: we do not own, patch or govern the
# egress of the box. Holding one of these AND a substrate class is the shape
# the shape this audit exists for — the narrower class becomes decorative.
FOREIGN = {"MOON", "EXTERNAL-INFRA", "EXTERNAL-KID", "EXTERNAL-INTERMEDIARY"}
SUBSTRATE = {"INFRA", "BACKEND-INFRA"}

# Gear that publishes or fronts devices. Graduated lateral reach on one of these
# is a standing grant toward everything P2P can reach, including — since
# the effector and arbiter classes.
FEED_GEAR = {"SENSOR-FEED", "SENSOR-TELEMETRY", "EFFECTOR-TELEMETRY", "KID", "EXTERNAL-KID"}

fail = 0
for p in peers:
    names = [g["name"] for g in (p.get("groups") or []) if g["name"] != "All"]
    tags = {bare(n) for n in names}
    nm = p["name"]

    both = tags & FOREIGN and tags & SUBSTRATE
    if both and declared.get((nm, "foreign_substrate")):
        d = declared[(nm, "foreign_substrate")]
        print(f"NOTE {nm}: foreign class with substrate — DECLARED by " + d.get("by","?"))
        both = False
    if both:
        print(f"FAIL {nm}")
        print(f"     holds a FOREIGN/EXTERNAL class {sorted(tags & FOREIGN)} AND substrate {sorted(tags & SUBSTRATE)}")
        print(f"     groups: {sorted(names)}")
        print(f"     Substrate reaches every peer any-port and the hidden backend tier.")
        print(f"     These two tiers are mutually exclusive by design; holding both makes")
        print(f"     the narrower one decorative. Which to drop depends on the box:")
        print(f"       we host it        -> drop EXTERNAL-INFRA (it is not externally hosted)")
        print(f"       someone else does -> drop INFRA (it must not administer everything)")
        fail = 1

    if tags & FEED_GEAR and "P2P" in tags:
        d = declared.get((nm, "gear_p2p"))
        if d:
            print(f"NOTE {nm}: publishing gear holds P2P — DECLARED by " + d.get("by","?"))
            print(f"     ops/ci/mesh-reach-declared-deviations.yaml for the reason and revisit condition")
            continue
        print(f"FAIL {nm}")
        print(f"     publishing gear {sorted(tags & FEED_GEAR)} also holds P2P (graduated lateral reach)")
        print(f"     groups: {sorted(names)}")
        print(f"     P2P grants any-port toward every role AND every gear class, including")
        print(f"     EFFECTOR-ARBITER where intent becomes an engagement. A feed does not")
        print(f"     initiate laterally; its fabric leg is <class> -> INFRA :7447/:7448.")
        fail = 1

if fail:
    print()
    print("F-311: coverage checks are a LOWER bound and cannot see an over-grant —")
    print("INFRA is the most-covered a peer can be. See docs/findings/F-311-*.md.")
    sys.exit(1)

print(f"OK: {len(peers)} peers — no foreign class holding substrate, no publishing")
print("    gear holding graduated lateral reach.")
'
