#!/usr/bin/env bash
# ADR 1116 §7 — the live-membership guard.
#
# Node-class groups hold GEAR. Role groups hold PEOPLE. Documentation does not
# enforce that:
#
#   - A node-class group documented as "machines running missions, not people"
#     can hold peers that are end-user desktops. A
#     `platform-admin -> <class> (any port)` rule written against the DOCUMENTED
#     meaning then grants admins any-port reach into a colleague's laptop.
#   - A human role can be added to an effector box because its hostname looks
#     like a person's peer.
#
# The definitions can be correct and still violated. Only a check that reads LIVE
# membership catches this.
#
# Usage:
#   NB_API_URL=https://netbird.<zone> NB_API_TOKEN=... \
#     ops/ci/check-node-class-membership.sh
#
# Optional mint-provenance stage (ADR 1116 §9 / machine-mesh-alignment D6b):
# when PORTAL_URL + PORTAL_OPERATOR_KEY are set, every peer holding a gear-class
# group is graded against the portal registry (GET /api/machines; join key =
# portal system_handle vs NetBird peer name) into exactly one bucket:
#
#   aligned            a live registry machine row matches the peer name
#   pending-reconcile  no row, but the peer is named in the optional allowlist
#                      file at PROVENANCE_RECONCILE_ALLOWLIST (one peer name per
#                      line, # comments) — the D6b offline-bundle / break-glass
#                      lane, retro-stamped at next portal availability
#   violation          no row, no allowlist entry — a side-door enrollment,
#                      OR a peer renamed off its handle (the join key is the
#                      NAME): when any registry handle matches no peer at all,
#                      the stage names those handles as the likely cause before
#                      the reader goes hunting for a mint that did happen
#   unknown            portal unreachable / API error mid-run; NEVER silently
#                      counted as aligned
#
# Report-only by default: exit 1 on violations only when PROVENANCE_ENFORCE=1
# (the plan's Phase 4 flips enforcement once the fleet is backfilled). Without
# PORTAL_URL the stage is skipped with one line saying so — PR CI cannot reach
# the mesh-internal portal; the stage runs from operator/mgmt-host contexts.
#
# NOT wired into PR CI: it needs live mesh credentials, and a check that passes
# silently because it could not see the truth is worse than no check. Run it
# from an operator shell or a credentialed scheduled job.
set -euo pipefail

: "${NB_API_URL:?set NB_API_URL, e.g. https://netbird.zagros-prod.example}"
: "${NB_API_TOKEN:?set NB_API_TOKEN (a NetBird PAT)}"

# Scope prefixes are SOURCED from the taxonomy (netbird_acl_taxonomy +
# netbird_exercises site_prefix values), not hardcoded — the previous five-entry
# list here was a fourth vocabulary copy waiting to drift the way the NODE_CLASS
# set did (CONTROL).
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TAXONOMY="${REPO_ROOT}/ops/ansible/netbird_acl_taxonomy.yml"
if [[ ! -f "$TAXONOMY" ]]; then
  echo "FAIL: missing ${TAXONOMY} — scope prefixes are sourced from the taxonomy" >&2
  exit 1
fi

peers="$(curl -sf -H "Authorization: Token ${NB_API_TOKEN}" "${NB_API_URL}/api/peers")" || {
  echo "FAIL: could not read ${NB_API_URL}/api/peers — refusing to pass on an unread mesh" >&2
  exit 1
}

# Optional mint-provenance inputs. Fetch failure grades every gear peer
# `unknown` rather than aborting: the membership checks above the provenance
# stage are still worth running on a mesh we CAN read.
machines_file=""
provenance_status="skipped"
if [[ -n "${PORTAL_URL:-}" && -n "${PORTAL_OPERATOR_KEY:-}" ]]; then
  machines_file="$(mktemp)"
  trap 'rm -f "$machines_file"' EXIT
  if curl -sf -H "Authorization: Bearer ${PORTAL_OPERATOR_KEY}" \
      "${PORTAL_URL%/}/api/machines" -o "$machines_file"; then
    provenance_status="loaded"
  else
    provenance_status="error"
  fi
fi

echo "$peers" | \
  GUARD_TAXONOMY="$TAXONOMY" \
  GUARD_PROVENANCE_STATUS="$provenance_status" \
  GUARD_MACHINES_FILE="$machines_file" \
  GUARD_RECONCILE_ALLOWLIST="${PROVENANCE_RECONCILE_ALLOWLIST:-}" \
  GUARD_PROVENANCE_ENFORCE="${PROVENANCE_ENFORCE:-0}" \
  GUARD_WARRANT_DIR="${WARRANT_MEMBERSHIP_DIR:-}" \
  python3 -c '
import json, os, re, sys

peers = json.load(sys.stdin)

# Gear classes (ADR 1116) + the older node classes. A peer in one of these is
# asserting "I am equipment", and equipment is not a person.
NODE_CLASS = {
    "SENSOR", "SENSOR-FEED", "SENSOR-TASKING", "SENSOR-TELEMETRY", "SENSOR-ARBITER",
    "EFFECTOR", "EFFECTOR-CUE", "EFFECTOR-TRACK", "EFFECTOR-TELEMETRY",
    "EFFECTOR-COMMAND", "EFFECTOR-ARBITER",
    "KID", "EXTERNAL-KID", "EXTERNAL-INTERMEDIARY", "FUSION", "CONTROL",
    "POD", "NETWORK", "MOON", "BREAKGLASS", "INFRA", "EXTERNAL-INFRA", "BACKEND-INFRA",
    "NETWORK-JUMP", "EDGE-SERVER", "PEERING-EDGE",
}
# NET-ENGINEER (ADR 1216) is a PERSON group: network engineers laptops, never
# gear. Counted with the roles so that (b) fails gear wearing it.
ROLE = {"PLATFORM-ADMIN", "MONITOR", "DEVELOPER", "OBSERVER", "NET-ENGINEER"}

# The ADR 1116 GEAR subset of NODE_CLASS — derived, not a fourth copy: the
# substrate/node classes are subtracted so growing the gear vocabulary in
# NODE_CLASS (parity-checked by check-gear-class-parity.py) grows this too.
SUBSTRATE_CLASS = {"POD", "NETWORK", "MOON", "INFRA", "EXTERNAL-INFRA", "BACKEND-INFRA",
                   # BREAKGLASS is the ADR 1114 emergency-access node — infrastructure,
                   # not gear. A correctly-configured break-glass peer holds no
                   # custody edge and never will, so without this entry it grades
                   # `violation` and enforcement fails the guard on right config.
                   "BREAKGLASS",
                   # NETWORK-JUMP (ADR 1131) is infrastructure for the same
                   # reason: it is the egress that administers bearer gear, it
                   # holds no custody edge and never will, and it is not a
                   # fabric source. Classing it as gear would grade a correctly
                   # configured jump host a violation, which is the BREAKGLASS
                   # mistake one class later.
                   "NETWORK-JUMP",
                   # EDGE-SERVER (ADR 1216) takes the NETWORK shape exactly: a
                   # site server that supports edge devices, administered over
                   # the mesh, holding no fabric ports and no custody edge.
                   "EDGE-SERVER",
                   # PEERING-EDGE (ADR 1219) is a federation edge router kept on
                   # the mesh: it dials the core router and nothing else, and is
                   # placed by the declaration, not minted, so it has no custody
                   # edge either.
                   "PEERING-EDGE"}
GEAR_CLASS = NODE_CLASS - SUBSTRATE_CLASS

# Scope prefixes from the taxonomy: goatnet entries + exercise/site stamps.
try:
    import yaml
except ImportError:
    print("FAIL: python3 yaml module not available (pip install pyyaml) — scope")
    print("      prefixes are sourced from netbird_acl_taxonomy.yml")
    sys.exit(1)
with open(os.environ["GUARD_TAXONOMY"]) as fh:
    tax = yaml.safe_load(fh) or {}
PREFIXES = []
for entry in (tax.get("netbird_acl_taxonomy") or {}).values():
    p = (entry or {}).get("site_prefix")
    if p:
        PREFIXES.append(p + "-")
for entry in (tax.get("netbird_exercises") or {}).values():
    p = (entry or {}).get("site_prefix")
    if p:
        PREFIXES.append(p + "-")
# Longest first, so <GOATNET>-<ENV>- strips before <GOATNET>- gets a chance to.
PREFIXES.sort(key=len, reverse=True)
if not PREFIXES:
    print("FAIL: no site_prefix values found in the taxonomy — cannot strip scopes")
    sys.exit(1)

# The rails mint human peers as peer-<handle>-<env>-<date>-<hostname>. That
# prefix is the only reliable human signal — hostnames are NOT, which is the
# whole lesson: DESKTOP-EQHQBIQ is an effector navigator box, and it collided with a
# person peer because cloned Windows images share a hostname.
HUMAN_NAME = re.compile(r"^peer-[a-z0-9._-]+-\w+-\d{4}-\d{2}-\d{2}")

# A MACHINE peer name is not its handle. The portal
# mints the setup key as "machine-<handle>" and sends that as auto_peer_name,
# which the patched mgmt image renders as "<auto_peer_name>-<hostname>". So a
# machine enrolled as svn-1 on a box called pi-1 arrives as
# machine-svn-1-pi-1.
#
# Exact equality (`nm in handles`) would grade EVERY mint-provisioned peer
# violation unless a human renamed it by hand, which makes the headline number a
# report on whether somebody remembered a manual step, not on whether the mint was
# the door.
#
# Match the documented shape instead, exactly as HUMAN_NAME above already does
# for people. Three forms are accepted and no others:
#
#   <handle>                      renamed to the bare handle by hand
#   machine-<handle>              auto_peer_name with no hostname appended
#   machine-<handle>-<hostname>   what the mgmt image actually produces
#
# The trailing hyphen matters: without it handle svn-1 would also claim
# machine-svn-11-thor, grading a DIFFERENT machine as aligned. A false pass
# is worse here than a false violation.
#
# NOTE FOR EDITORS: this python runs inside a single-quoted shell string, so an
# apostrophe anywhere in this block terminates it and the script dies with a
# syntax error far from the cause.
def owning_handle(peer_name, handles):
    for h in handles:
        base = "machine-" + h
        if peer_name == h or peer_name == base or peer_name.startswith(base + "-"):
            return h
    return None

def bare(g):
    # strip <GOATNET>- / <SITE>- prefixes: GOATNET-SENSOR-FEED -> SENSOR-FEED
    for p in PREFIXES:
        if g.startswith(p):
            return g[len(p):]
    return g

# External-service hosts are minted under a person (F-531), so their names read
# as human. Warrant declares them as exact: [<P>-EXTERNAL-INFRA] (ADR 1178), and a
# host declared that way may hold the class. Holding anything else still fails.
def declared_external_hosts(root):
    if not root or not os.path.isdir(root):
        return set()
    import glob
    import yaml
    out = set()
    for f in glob.glob(os.path.join(root, "*.yaml")):
        if os.path.basename(f).startswith("_"):
            continue
        with open(f) as fh:
            doc = yaml.safe_load(fh) or {}
        for e in doc.get("entries") or []:
            exact = e.get("exact") or []
            if len(exact) == 1 and exact[0].endswith("-EXTERNAL-INFRA"):
                out.add(e.get("peer"))
    return out

DECLARED_EXTERNAL = declared_external_hosts(os.environ.get("GUARD_WARRANT_DIR", ""))

fail = 0
gear_peers = []
for p in peers:
    names = [g["name"] for g in (p.get("groups") or []) if g["name"] != "All"]
    tags  = {bare(n) for n in names}
    classes = tags & NODE_CLASS
    roles   = tags & ROLE
    human   = bool(HUMAN_NAME.match(p["name"]))
    nm      = p["name"]

    if tags & GEAR_CLASS:
        gear_peers.append(nm)

    # (a) a person-shaped peer must not be wearing a gear class
    if human and classes and nm not in DECLARED_EXTERNAL:
        print(f"FAIL {nm}")
        print(f"     rails-minted human peer holds NODE CLASS {sorted(classes)}")
        print(f"     Node classes are for equipment. Give the person a role group instead.")
        fail = 1

    # (b) gear must not be wearing a person role
    if classes and roles:
        print(f"FAIL {nm}")
        print(f"     gear ({sorted(classes)}) also holds PERSON ROLE {sorted(roles)}")
        print(f"     A login role on equipment is how a cUAS ground segment ended up")
        print(f"     with OBSERVER on 2026-08-09. Remove the role.")
        fail = 1

    # (c) covered by nothing at all — invisible to every rule, and to any audit
    # A quarantined or probation peer is classified: warrant holds it there.
    held = any(t == "QUARANTINED" or t.startswith("PROBATION") for t in tags)
    if not classes and not roles and not held:
        print(f"FAIL {nm}")
        shown = names if names else ["[only All]"]
        print(f"     holds neither a node class nor a role: {shown}")
        print(f"     Unclassified peers are what the OPSYSTEM catch-all absorbed.")
        fail = 1

# --- mint-provenance stage (ADR 1116 §9 / D6b) ------------------------------
prov_fail = 0
status = os.environ.get("GUARD_PROVENANCE_STATUS", "skipped")
print()
if status == "skipped":
    print("mint provenance: SKIPPED — PORTAL_URL / PORTAL_OPERATOR_KEY not set")
    print("  (the portal is mesh-internal; run this stage from an operator or mgmt-host shell)")
else:
    allowlist = set()
    al_path = os.environ.get("GUARD_RECONCILE_ALLOWLIST", "")
    if al_path:
        with open(al_path) as fh:
            for line in fh:
                line = line.strip()
                if line and not line.startswith("#"):
                    allowlist.add(line)

    handles = set()
    if status == "loaded":
        try:
            with open(os.environ["GUARD_MACHINES_FILE"]) as fh:
                data = json.load(fh)
            handles = {m.get("system_handle", "") for m in data.get("machines", [])} - {""}
        except (OSError, ValueError):
            status = "error"

    buckets = {"aligned": [], "pending-reconcile": [], "violation": [], "unknown": []}
    for nm in gear_peers:
        if status != "loaded":
            buckets["unknown"].append(nm)          # NEVER silently counted as aligned
        elif owning_handle(nm, handles):
            buckets["aligned"].append(nm)
        elif nm in allowlist:
            buckets["pending-reconcile"].append(nm)
        else:
            buckets["violation"].append(nm)

    total = len(gear_peers)
    print(f"mint provenance ({total} gear-class peers):")
    if status == "error":
        print("  portal unreachable or /api/machines error — every gear peer grades UNKNOWN")
    for b in ("aligned", "pending-reconcile", "violation", "unknown"):
        print(f"  {b:<18}{len(buckets[b])}/{total}")
        if b != "aligned":
            for nm in sorted(buckets[b]):
                print(f"    - {nm}")
    enforce = os.environ.get("GUARD_PROVENANCE_ENFORCE", "0") == "1"
    if buckets["violation"]:
        # The join is peer NAME == registry HANDLE, so a peer renamed off its
        # handle grades `violation` even though its custody edge exists and is
        # perfectly good. So a peer with an appliance hostname must be renamed to
        # its handle.
        #
        # A registry handle that matches NO peer at all is the signature. Name
        # the unmatched handles next to the violations so the reader can pair
        # them up, instead of concluding "side-door enrollment" and going
        # looking for a mint that did happen.
        all_peer_names = {p["name"] for p in peers}
        orphan_handles = sorted(handles - all_peer_names)
        if orphan_handles:
            print()
            print("  LIKELY CAUSE — name/handle mismatch, not a side door:")
            print(f"  {len(orphan_handles)} registry machine handle(s) match no peer name at all:")
            for h in orphan_handles:
                print(f"    - {h}")
            print("  The provenance join key is `peer name == machine handle`. A peer left on")
            print("  its appliance hostname reads as a violation while its custody edge is")
            print("  fine. Check whether a violation above is one of these machines under a")
            print("  different name; if so RENAME THE PEER to the handle (name-only change —")
            print("  groups, addresses and connectivity are untouched) rather than allowlisting")
            print("  it. Only after that does a remaining violation mean a real side door.")
        if enforce:
            print("  FAIL: violations present under PROVENANCE_ENFORCE=1 — a peer wearing a")
            print("  gear class with no registry custody edge did not come through the mint.")
            prov_fail = 1
        else:
            print("  report-only: violations present; set PROVENANCE_ENFORCE=1 to fail on them")
            print("  (enforcement flips in machine-mesh-alignment Phase 4, after backfill)")

if fail:
    print()
    print("ADR 1116 §7: node-class groups hold gear, role groups hold people,")
    print("and every peer is one or the other. See docs/adr/1116-*.md.")
    sys.exit(1)
if prov_fail:
    sys.exit(1)

print()
print(f"OK: {len(peers)} peers — no gear wearing a person role, no person wearing a")
print("    gear class, nothing unclassified.")
'
