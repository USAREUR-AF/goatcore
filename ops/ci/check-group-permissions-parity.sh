#!/usr/bin/env bash
# ops/ci/check-group-permissions-parity.sh — the ADR 1073 parity ratchet.
#
# ops/permissions/group-permissions.yml is the ENFORCED authorization matrix.
# For every service whose front door is an oauth2-proxy allowed-groups fence,
# the deployed allowed-groups list (ansible role default + every group_vars
# override) must equal the set of groups the matrix grants that service.
# Divergence is a build failure, not a doc footnote.
#
# Fail-closed ratchet (the check-rollback-coverage.sh pattern): the COVERED
# map below may only grow. A new oauth2-proxy fence must be added here in the
# same change that adds its role, or this script's self-audit fails.
#
# Scope note: an oauth2-proxy fence is a BINARY door — this check admits any
# group with a non-none grant, so a `read`-level group passes the same door
# an `admin` group does. Read-vs-admin differentiation is the app's job
# (matrix-consulting in-app gates, the id-console pattern). Fine for
# view-only surfaces; a future covered service with mutation routes MUST
# pair the door with an in-app level check.
set -euo pipefail

PERMS="${1:-ops/permissions/group-permissions.yml}"
ANSIBLE_DIR="${2:-ops/ansible}"

python3 - "$PERMS" "$ANSIBLE_DIR" <<'PY'
import glob, re, sys, yaml

perms_path, ansible_dir = sys.argv[1], sys.argv[2]
matrix = yaml.safe_load(open(perms_path))

# matrix-service -> the ansible var carrying that service's oauth2-proxy
# allowed-groups list. Coverage may only grow.
COVERED = {
    # herd: the door is a group gate, but unlike every other covered service the
    # door is NOT the authorization layer here — the ADR 1131 network_admin
    # entitlement is, checked per session. This entry covers the door only.
    "herd": "herd_oauth2_proxy_allowed_groups",
    "netbird-dashboard": "dashboard_oauth2_proxy_allowed_groups",
    # The admin address of mesh change control (ADR 1189): a second
    # oauth2-proxy-dashboard instance admitting only NETBIRD-ADMIN.
    "netbird-dashboard-admin": "dashboard_admin_oauth2_proxy_allowed_groups",
    "id-admin-console": "id_admin_console_oauth2_proxy_allowed_groups",
    "relay-health": "relay_health_oauth2_proxy_allowed_groups",
    "compliance-dashboard": "compliance_proxy_allowed_groups",
    "microscope": "microscope_oauth2_proxy_allowed_groups",
    "netvis": "netvis_oauth2_proxy_allowed_groups",
    "consistency-console": "consistency_console_oauth2_proxy_allowed_groups",
    "consistency-aggregator": "consistency_aggregator_oauth2_proxy_allowed_groups",
    # Authenticated-door surfaces. Registered here on purpose even though their
    # door admits everyone: the ratchet's job is that no fence var exists
    # unregistered, and "any authenticated" must be a DECLARED posture rather
    # than an absent knob. panoscope is the cautionary case — it had no knob,
    # no fence container, and served HTTP 200 to any caller on production.
    "panoscope": "panoscope_oauth2_proxy_allowed_groups",
    "isitup": "isitup_oauth2_proxy_allowed_groups",
    # Authenticated doors (oauth2-proxy-surface) on airdefenders / brief /
    # dataflows: built taken down (#726, deployed ahead
    # of review), put back the same day through review. The door refuses
    # nobody; it exists so Hydra records who reached the surface. These rows
    # are valid only while the door is ON somewhere — the dormant-door rule
    # below fails the build if it is off everywhere.
    "airdefenders": "airdefenders_oauth2_proxy_allowed_groups",
    "brief": "brief_oauth2_proxy_allowed_groups",
    "dataflows": "dataflows_oauth2_proxy_allowed_groups",
}

# Every service MUST declare a door, from a closed set. The door says what the
# FRONT DOOR decides, which is what tells a reader how to read a blank cell in
# the console's group->service grid (see the header of group-permissions.yml).
#
# This is required rather than optional because a service with no door and a
# `fence:` claiming an oauth2-proxy that does not exist renders in the console as
# denied to every group but PLATFORM-ADMIN while nobody is gated. An absent knob is
# how a surface reaches production serving 200 to any caller.
DOOR_VALUES = {"group-gated", "authenticated", "in-app", "mesh-only"}

# Doors that are NOT an oauth2-proxy group fence. A service declaring one of
# these must not be registered in COVERED, and must own no allowed-groups var:
# the ratchet would otherwise "verify" a fence that isn't there.
NON_PROXY_DOORS = {"in-app", "mesh-only"}

doors = {}
for s in matrix["services"]:
    d = str(s.get("door", "")).strip()
    if not d:
        print(f"FAIL: {s['name']}: no door declared — every service must declare one of {sorted(DOOR_VALUES)}")
        ok = False
    elif d not in DOOR_VALUES:
        print(f"FAIL: {s['name']}: door {d!r} is not one of {sorted(DOOR_VALUES)}")
        ok = False
    doors[s["name"]] = d

ok = all(d in DOOR_VALUES for d in doors.values())

# A grant on a mesh-only door enforces nothing — no fence reads it — and the
# console renders it as an access list: a `play` grant to
# PLATFORM-ADMIN on airdefenders read as "nobody else may play" while the
# surface admitted every mesh caller. So a mesh-only service holds no grants.
for g in matrix["groups"]:
    for gr in g.get("grants", []):
        if doors.get(gr["service"]) == "mesh-only":
            print(f"FAIL: {g['name']} grants {gr['access']!r} on {gr['service']}, whose door is mesh-only — nothing enforces it; drop the row or fence the surface")
            ok = False

for service, door in sorted(doors.items()):
    if door in NON_PROXY_DOORS and service in COVERED:
        print(f"FAIL: {service}: door {door!r} is not an oauth2-proxy fence, so it must not be in COVERED")
        ok = False
    if door not in NON_PROXY_DOORS and service not in COVERED:
        print(f"FAIL: {service}: door {door!r} is an oauth2-proxy fence but the service is not in COVERED")
        ok = False

# Services whose FENCE is deliberately not the authorization boundary: the door
# admits any authenticated identity and the app differentiates. Declared in the
# matrix so it is a decision on the record, never an absent knob.
authenticated_door = {n for n, d in doors.items() if d == "authenticated"}

def granting_groups(service):
    out = set()
    for g in matrix["groups"]:
        for gr in g.get("grants", []):
            if gr["service"] == service and gr["access"] not in ("", "none"):
                out.add(g["name"])
    return out

def var_assignments(var):
    """Every literal assignment of `var`: [(path, value, is_group_vars)]."""
    hits = []
    paths = [(p, False) for p in glob.glob(f"{ansible_dir}/roles/*/defaults/main.yml")] + [
        (p, True) for p in glob.glob(f"{ansible_dir}/group_vars/*/*.yml")
    ]
    pat = re.compile(rf'^\s*{re.escape(var)}\s*:\s*["\']?([^"\'#\n]*)["\']?\s*(#.*)?$')
    for p, is_gv in paths:
        try:
            for line in open(p, encoding="utf-8"):
                m = pat.match(line)
                if m:
                    hits.append((p, m.group(1).strip(), is_gv))
        except OSError:
            continue
    return hits

for service, var in sorted(COVERED.items()):
    prefix = var[: -len("oauth2_proxy_allowed_groups")]
    enabled_hits = var_assignments(prefix + "oauth2_proxy_enabled")
    if enabled_hits and not any(v.strip().lower() in ("true", "yes", "on") for _, v, _ in enabled_hits):
        print(f"FAIL: {service}: the matrix declares door: {doors.get(service)!r} and the fence is COVERED, but {prefix}oauth2_proxy_enabled is false on every goatnet — the door runs nowhere; set the matrix to what runs")
        ok = False
        continue
    want = granting_groups(service)
    if not want and service in authenticated_door:
        # An authenticated door is the whole decision: any login, no group
        # refused. Grants are optional there and exist only where the app
        # differentiates behind the door (relay-health). A lone grant on a
        # surface that does not differentiate renders as an access list
        # ("PLATFORM-ADMIN plays, nobody else does"), so zero is the honest
        # count for airdefenders / brief / dataflows.
        print(f"note: {service}: authenticated door, no grants — the door is the whole decision")
    elif not want:
        print(f"FAIL: {service} is covered but the matrix grants it to no group")
        ok = False
        continue
    hits = var_assignments(var)
    if not hits:
        print(f"FAIL: {service}: no assignment of {var} found under {ansible_dir}")
        ok = False
        continue
    for path, value, is_group_vars in hits:
        got = set(x.strip() for x in value.split(",") if x.strip())
        if not is_group_vars:
            # Role defaults are the canonical posture: strict equality —
            # UNLESS the matrix declares the surface's door authenticated-only.
            #
            # relay-health is the case that forced this: the sensitive thing is
            # the DETAIL, not the existence of the page, so the door admits any
            # authenticated identity and nginx serves the full or the reduced
            # document from the caller's groups (ADR 1066, fail-closed to
            # reduced). Gating the door on an admin group instead made it
            # all-or-nothing and handed IAM-ADMIN full relay internals it has no
            # operational need for. The `grants` below still record who may read
            # WHAT; `door: authenticated` records that the fence is not where
            # that decision is made.
            if service in authenticated_door:
                if got:
                    print(f"FAIL: {service}: the matrix declares door: authenticated, so {path} must leave {var} empty (found {sorted(got)})")
                    ok = False
                else:
                    print(f"note: {service}: authenticated-only door by matrix declaration; in-app tiers are the authorization layer")
                continue
            if got != want:
                print(f"FAIL: {service}: {path} sets {var}={sorted(got)} but the matrix grants {sorted(want)}")
                ok = False
        else:
            # group_vars overrides are per-goatnet. Two legitimate shapes,
            # both surfaced, never silent:
            #   EMPTY  — a deliberate authenticated-only door (the ADR 1072
            #            sponsor pattern: partner identities carry no Ory
            #            groups, so the app is the authorization layer).
            #   WIDER  — a superset of the matrix set (never silently drop
            #            a matrix-granted group); extras are per-goatnet
            #            door groups the in-app matrix gate bounds.
            if not got:
                print(f"note: {service}: {path} sets an authenticated-only door (empty {var}); the in-app matrix gate is the authorization layer")
                continue
            missing = want - got
            if missing:
                print(f"FAIL: {service}: {path} {var} is missing matrix-granted {sorted(missing)}")
                ok = False
            extras = got - want
            if extras:
                print(f"note: {service}: {path} widens the door with per-goatnet groups {sorted(extras)} (in-app gates bound them)")

# Self-audit: any *oauth2_proxy_allowed_groups var in the tree that is not
# covered above is an uncovered fence — the ratchet refuses silently-new gates.
known_vars = set(COVERED.values())
uncovered = set()
audit_paths = glob.glob(f"{ansible_dir}/roles/*/defaults/main.yml") + glob.glob(
    f"{ansible_dir}/group_vars/*/*.yml"
)
dormant = {}
for p in audit_paths:
    for line in open(p, encoding="utf-8"):
        m = re.match(r"^\s*([a-z0-9_]*)oauth2_proxy_allowed_groups\s*:", line)
        if not m or m.group(1) + "oauth2_proxy_allowed_groups" in known_vars:
            continue
        var = m.group(1) + "oauth2_proxy_allowed_groups"
        # A DORMANT door: the role ships <prefix>_oauth2_proxy_enabled and no
        # goatnet turns it on. airdefenders / brief / dataflows are this shape
        # — the door exists in the tree and runs nowhere, so
        # the matrix must say what runs (mesh-only), not what could. The var
        # is admitted only while BOTH hold: enabled is false everywhere AND the
        # matrix door is not a proxy fence. Flip enabled on one goatnet without
        # touching the matrix and this fails; declare the matrix door
        # authenticated with the door still off everywhere and this fails.
        service = m.group(1).rstrip("_").replace("_", "-")
        enabled_hits = var_assignments(m.group(1) + "oauth2_proxy_enabled")
        on_at = [path for path, value, _ in enabled_hits if value.strip().lower() in ("true", "yes", "on")]
        if enabled_hits and not on_at and doors.get(service) in NON_PROXY_DOORS:
            dormant[service] = var
            continue
        if on_at:
            uncovered.add(f"{var} ({p}) — door is ON in {sorted(on_at)} but the matrix says door: {doors.get(service)!r} and it is not COVERED")
        else:
            uncovered.add(f"{var} ({p})")
if uncovered:
    print("FAIL: oauth2-proxy fence vars not covered by this ratchet — add them to COVERED + the matrix:")
    for u in sorted(uncovered):
        print(f"  {u}")
    ok = False
for service, var in sorted(dormant.items()):
    print(f"note: {service}: dormant door ({var} present, *_oauth2_proxy_enabled false on every goatnet); matrix door {doors[service]!r} describes what runs")

# Panoscope platform-implication set (ADR 1074): panoscope_rbac_full_groups
# names the platform groups whose declared implication is data.view.all at
# Full strength — it must equal the matrix's panoscope `read` grants (the
# `scoped` level rides the data.* grammar and is outside this check).
pano_want = granting_groups("panoscope") - {
    g["name"] for g in matrix["groups"]
    for gr in g.get("grants", [])
    if gr["service"] == "panoscope" and gr["access"] == "scoped"
}
if not var_assignments("panoscope_rbac_full_groups"):
    print(f"FAIL: panoscope: no assignment of panoscope_rbac_full_groups under {ansible_dir}")
    ok = False
tier_union = set()
for path, value, _ in var_assignments("panoscope_rbac_full_groups"):
    tier_union |= {x.strip() for x in value.split(",") if x.strip()}
if tier_union and tier_union != pano_want:
    print(
        f"FAIL: panoscope: panoscope_rbac_full_groups grants {sorted(tier_union)} but the "
        f"matrix grants panoscope read to {sorted(pano_want)} — change the matrix first"
    )
    ok = False

if not ok:
    sys.exit(1)
print(
    f"ok: {len(COVERED)} oauth2-proxy fences + panoscope viewer tiers match {perms_path}; "
    "no uncovered fence vars"
)
PY
