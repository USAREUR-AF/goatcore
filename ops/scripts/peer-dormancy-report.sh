#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# peer-dormancy-report.sh — who ADR 1136 would quarantine or revoke, and why.
#
# REPORT ONLY. It changes nothing. ADR 1136 D6 requires the first run of
# anything that removes membership to publish the roster it would act on first,
# because silently disconnecting people is indistinguishable from an outage and
# the affected owners are the least likely to be watching.
#
# THRESHOLDS (ADR 1136 D2), which differ by node class because the failure modes
# differ:
#
#   people  quarantine 14d   revoke 60d
#   gear    quarantine 30d   revoke NEVER
#
# Gear is never automatically revoked because re-onboarding requires reaching
# the device, which is exactly what is impossible for something that has been
# dark for a month. The system must not be able to strand its own hardware.
#
# CLASS RESOLUTION (ADR 1136 D3): read from the registry, never inferred from
# the peer name. A peer whose class cannot be determined is treated as GEAR —
# fail toward the reversible action.
#
# usage:
#   ops/scripts/peer-dormancy-report.sh <goatnet>          # human roster
#   ops/scripts/peer-dormancy-report.sh <goatnet> --csv
#
# <goatnet> names the goatnet (site-a or site_a). Everything else is read from
# what that goatnet's configuration declares (ops/scripts/goatnet-facts.py):
#   the management host   user@mesh_ip of its first management host in
#                         ops/ansible/inventory/doors.yaml; MGMT_HOST=user@host
#                         names another
#   a person's device     a peer named peer-<handle>-<goatnet>-<YYYY-MM-DD>-...,
#                         the name the onboarding setup key gives it
#   infrastructure        a peer whose name starts with one of the function
#                         prefixes below, or with an item of the goatnet's
#                         peer_report_infra_prefixes (group_vars) list
#
# env:
#   MGMT_HOST     user@host of the management host (default: from doors.yaml)
#   PG_CONTAINER  postgres container name (default postgres-a)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GOATNET="${1:-}"
case "$GOATNET" in ""|-*) echo "usage: $(basename "$0") <goatnet> [--csv]" >&2; exit 2 ;; esac
MODE="${2:-report}"
PG_CONTAINER="${PG_CONTAINER:-postgres-a}"
MGMT_HOST="${MGMT_HOST:-$(python3 "$HERE/goatnet-facts.py" mgmt-ssh "$GOATNET")}" \
  || { echo "peer-dormancy-report: no management host for $GOATNET; set MGMT_HOST=user@host" >&2; exit 2; }
INFRA_EXTRA="$(python3 "$HERE/goatnet-facts.py" var "$GOATNET" peer_report_infra_prefixes 2>/dev/null | paste -sd' ' - || true)"

tmp="$(mktemp)"
cat > "$tmp" <<'SQL'
\pset format unaligned
\pset fieldsep '|'
\pset tuples_only on
SELECT name,
       CASE WHEN peer_status_connected THEN 'UP' ELSE 'down' END,
       EXTRACT(day FROM (now() - peer_status_last_seen))::int,
       meta_go_os,
       COALESCE(NULLIF(user_id,''),''),
       (meta_flags::json->>'RosenpassEnabled')
FROM peers
ORDER BY peer_status_last_seen ASC;
SQL
# 0600 from mktemp is unreadable by the postgres uid inside the container, and
# the resulting "Permission denied" goes to stderr where it is easy to swallow
# and report a confident empty roster.
chmod 644 "$tmp"

scp -q -o BatchMode=yes "$tmp" "${MGMT_HOST}:/tmp/dormancy.sql"
raw="$(ssh -o BatchMode=yes "${MGMT_HOST}" \
  "sudo docker cp /tmp/dormancy.sql ${PG_CONTAINER}:/tmp/d.sql >/dev/null 2>&1;
   sudo docker exec ${PG_CONTAINER} psql -U postgres -d netbird -f /tmp/d.sql 2>/dev/null;
   rm -f /tmp/dormancy.sql")"
rm -f "$tmp"

if [ -z "${raw//[[:space:]]/}" ]; then
  echo "peer-dormancy-report: FAILED to read the peer table (empty result)." >&2
  echo "  This is NOT an all-clear — nothing was evaluated." >&2
  exit 2
fi

python3 - "$MODE" "$GOATNET" "$INFRA_EXTRA" <<PYEOF
import sys, re, collections
mode, goatnet, infra_extra = sys.argv[1], sys.argv[2], sys.argv[3].split()
raw = """$raw"""

# Class resolution. The registry is authoritative; this script has only the peer
# table, so it applies the SAME fail-toward-gear rule ADR 1136 D3 requires:
# anything not positively identified as a person's device is treated as gear.
# Infrastructure named by function is neither — it is excluded entirely, because
# a mgmt host or a DNS node is not dormant-reapable by this policy at all.
INFRA = re.compile(r"^(mgmt-|mesh-dns-|relay-|collab-|goat-|goatnet-|ip-\d"
                   + "".join("|" + re.escape(p) for p in infra_extra) + ")")
PERSON = re.compile(r"^peer-.+-" + re.escape(goatnet.replace("_", "-")) + r"-\d{4}-\d{2}-\d{2}")

rows = []
for line in raw.splitlines():
    if line.count("|") < 5:
        continue
    name, state, days, os_, uid, rp = [c.strip() for c in line.split("|")][:6]
    try:
        days = int(days)
    except ValueError:
        continue
    if INFRA.match(name):
        cls = "infra"
    elif PERSON.match(name):
        cls = "person"
    else:
        cls = "gear"      # fail toward the reversible action
    rows.append((name, state, days, os_, cls, rp))

def verdict(cls, state, days):
    if state == "UP":
        return "active"
    if cls == "infra":
        return "exempt"
    if cls == "person":
        if days >= 60: return "REVOKE"
        if days >= 14: return "quarantine"
    else:
        if days >= 30: return "quarantine"
    return "active"

acted = [(n,s,d,o,c,rp,verdict(c,s,d)) for n,s,d,o,c,rp in rows]

if mode == "--csv":
    print("peer,class,state,days_since_seen,os,rosenpass,verdict")
    for n,s,d,o,c,rp,v in acted:
        print("%s,%s,%s,%d,%s,%s,%s" % (n,c,s,d,o,rp,v))
    raise SystemExit

counts = collections.Counter(v for *_ ,v in acted)
print()
print("  ADR 1136 dormancy evaluation — REPORT ONLY, nothing was changed")
print("  people: quarantine 14d / revoke 60d    gear: quarantine 30d / never revoke")
print()
print("  %-12s %d" % ("active", counts["active"]))
print("  %-12s %d" % ("exempt(infra)", counts["exempt"]))
print("  %-12s %d" % ("quarantine", counts["quarantine"]))
print("  %-12s %d" % ("REVOKE", counts["REVOKE"]))
print()
for want in ("REVOKE", "quarantine"):
    sel = [a for a in acted if a[6] == want]
    if not sel:
        print("  %s: none" % want)
        continue
    print("  %s (%d):" % (want, len(sel)))
    for n,s,d,o,c,rp,v in sorted(sel, key=lambda x:-x[2]):
        print("     %-46s %-7s %4dd  %-8s rp=%s" % (n[:46], c, d, o, rp))
    print()
print("  Gear is never auto-revoked: re-onboarding needs physical reach, which is")
print("  precisely what a device dark for a month does not have (ADR 1136 D2).")
PYEOF
