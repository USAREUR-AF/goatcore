#!/usr/bin/env bash
# netbird-admin-reconcile.sh — Kratos `groups` -> netbird-mgmt `role`, on a timer.
#
# THE ROLE MAP (docs/design/mesh-change-control.md §2):
#
# NETBIRD_MGMT_ROLE_MAP is an ordered, comma-separated list of GROUP=ROLE pairs,
# ROLE one of user | auditor | admin, e.g.
#   NETBIRD-ADMIN=admin,PLATFORM-ADMIN=auditor,IAM-ADMIN=auditor
# A person's entitled role is the HIGHEST role any of their Ory groups maps to
# (user < auditor < admin). When the map is unset it is derived from the older
# NETBIRD_MGMT_ADMIN_GROUPS (each group -> admin), which is exactly the
# behaviour every goatnet had before the map existed.
#
# WHY A TIMER AND NOT AN ONBOARDING STEP:
#
# NetBird separates two identities: a PEER (an enrolled device) and a USER (a
# dashboard login). Onboarding can mint the peer, but it cannot mint the user —
# we run without NetBird's IdP-manager integration, so `POST /api/users` returns
# 412 "idp manager must be enabled". A user row therefore comes into existence
# ONLY when the person completes their own first OIDC sign-in, and NetBird
# creates it as a plain `user`.
#
# That timing is the whole problem. An onboarding-time promotion has nothing to
# promote, and a reconciler that runs only during dashboard-deploy leaves an
# admin onboarded on Tuesday a non-admin until someone happens to deploy the
# dashboard: correct Ory groups, no NetBird rights, no signal.
#
# So the reconcile must be CONTINUOUS: it watches for the row to appear and
# promotes it within one timer interval, forever, with no operator in the loop.
#
# SAFETY — the two rules this script exists to never break:
#
#   1. It promotes ONLY up the ladder user -> auditor -> admin (from `user` or
#      `auditor`, never from anything else). It never touches `owner`. NetBird has
#      exactly one owner, and the natural-looking predicate `role != 'admin'`
#      MATCHES 'owner' — the previous inline version would have demoted the
#      account owner (who carries PLATFORM-ADMIN like any admin), leaving the
#      goatnet ownerless. Ownership can only be returned by the current owner,
#      so that is a self-inflicted lockout. Promote up from `user`, never sideways
#      from `owner`.
#   2. It is a role-only UPDATE. Never INSERT into netbird `users` — a partial
#      insert leaves NULLs that fail peer-login and drop the mesh.
#
# PROMOTE-ONLY, BY DESIGN — and it REPORTS the other direction. A person whose
# Ory groups stop mapping to admin keeps NetBird `admin` until someone demotes
# them; this timer never does, because an automatic demotion fired by a group
# typo, a map edit or a Kratos outage would strip every admin on the goatnet at
# once. The demotion is an explicit, audited step: `goat admin platform-admin
# revoke` (docs/design/platform-admin-lifecycle.md), or on a goatnet with a role
# map, ops/ansible/netbird-role-realign.yml. What this script does is make the
# gap VISIBLE: every human NetBird admin whose active Kratos identity does not
# map to admin is logged as STRAY and counted in
# netbird_admin_reconcile_stray_admins.
set -euo pipefail

KRATOS_ADMIN="${KRATOS_ADMIN_URL:-http://127.0.0.1:4434}"
ADMIN_GROUPS="${NETBIRD_MGMT_ADMIN_GROUPS:-PLATFORM-ADMIN}"
ROLE_MAP="${NETBIRD_MGMT_ROLE_MAP:-}"
TEXTFILE_DIR="${TEXTFILE_DIR:-/var/lib/node_exporter/textfile}"
PG_CONTAINER="${PG_CONTAINER:-postgres-a}"
PG_USER="${PG_USER:-postgres}"
NB_DB="${NB_DB:-netbird}"

log() { echo "[netbird-admin-reconcile] $*"; }

# No map: derive it from the admin-group list, every group -> admin. That is
# the pre-map behaviour, so a goatnet that never sets a map is unchanged.
if [ -z "$ROLE_MAP" ]; then
  ROLE_MAP=$(tr ',' '\n' <<<"$ADMIN_GROUPS" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' \
    | grep -v '^$' | sed 's/$/=admin/' | paste -sd, -)
fi

# user < auditor < admin. Anything else (owner, billing_admin, network_admin)
# is off the ladder and never moved by this script.
rank() {
  case "$1" in
    user) echo 1 ;;
    auditor) echo 2 ;;
    admin) echo 3 ;;
    *) echo 0 ;;
  esac
}

# --- 1. what role does Kratos say each person should hold? ------------------
identities_json=$(curl -sS -m 15 "${KRATOS_ADMIN}/admin/identities?per_page=500" 2>/dev/null) || {
  log "Kratos admin API unreachable at ${KRATOS_ADMIN} — nothing to do this tick"
  exit 0
}

# One row per active identity entitled to MORE than `user`:
#   <kratos id>\t<email>\t<role>
# A map that fails to parse prints nothing and exits 3, so a typo in the map
# promotes nobody rather than guessing.
entitled_rows=$(
  ROLE_MAP="$ROLE_MAP" python3 -c '
import json, os, sys

RANK = {"user": 1, "auditor": 2, "admin": 3}
role_map = []
for pair in os.environ["ROLE_MAP"].split(","):
    pair = pair.strip()
    if not pair:
        continue
    group, sep, role = pair.partition("=")
    group, role = group.strip(), role.strip()
    if not sep or not group or role not in RANK:
        print(f"bad role map entry {pair!r}", file=sys.stderr)
        sys.exit(3)
    role_map.append((group, role))
try:
    identities = json.loads(sys.stdin.read())
except Exception:
    sys.exit(0)
for i in identities:
    if i.get("state") != "active":
        continue
    traits = i.get("traits") or {}
    meta = i.get("metadata_public") or {}
    # Login groups come ONLY from admin-managed metadata_public (GHSA-9v3v):
    # self-editable traits never grant a role, even for a not-yet-backfilled
    # identity.
    groups = set(meta.get("groups") or [])
    best = "user"
    for group, role in role_map:
        if group in groups and RANK[role] > RANK[best]:
            best = role
    if best != "user":
        print("\t".join([i["id"], traits.get("email", ""), best]))
' <<<"$identities_json"
) || {
  log "ERROR: NETBIRD_MGMT_ROLE_MAP '${ROLE_MAP}' does not parse (GROUP=user|auditor|admin, comma-separated) — promoting nobody this tick"
  exit 1
}
want=()
[ -n "$entitled_rows" ] && mapfile -t want <<<"$entitled_rows"

n_admin=0
n_auditor=0
for row in "${want[@]}"; do
  case "${row##*$'\t'}" in
    admin) n_admin=$((n_admin + 1)) ;;
    auditor) n_auditor=$((n_auditor + 1)) ;;
  esac
done

# --- 1b. strays: human netbird admins the map does NOT entitle to admin -----
# Read before the promote loop so an empty entitled set still reports strays.
mapfile -t nb_admins < <(
  docker exec "$PG_CONTAINER" psql -U "$PG_USER" -d "$NB_DB" -tAc \
    "SELECT id FROM users WHERE role = 'admin' AND NOT COALESCE(is_service_user, false);" 2>/dev/null \
    | tr -d '[:blank:]' | grep -v '^$' || true
)
stray=0
for id in "${nb_admins[@]}"; do
  entitled=0
  for row in "${want[@]}"; do
    [ "${row%%$'\t'*}" = "$id" ] && [ "${row##*$'\t'}" = "admin" ] && { entitled=1; break; }
  done
  if [ "$entitled" -eq 0 ]; then
    stray=$((stray + 1))
    email=$(IDENTITY_ID="$id" python3 -c '
import json, os, sys
try:
    for i in json.loads(sys.stdin.read()):
        if i.get("id") == os.environ["IDENTITY_ID"]:
            print((i.get("traits") or {}).get("email", "")); break
except Exception:
    pass
' <<<"$identities_json")
    log "STRAY netbird admin ${email:-$id} (${id}): no active Kratos identity whose groups map to admin in [${ROLE_MAP}] — demote with: ansible-playbook netbird-role-realign.yml -e target=<site>_hosts -e apply=true (or goat admin platform-admin revoke ${email:-<email>} --env <goatnet> --apply)"
  fi
done

if [ "${#want[@]}" -eq 0 ]; then
  log "no active Kratos identity carries a group mapped above user in [${ROLE_MAP}] — nothing to promote (${stray} stray admin(s))"
fi

# --- 2. promote the ones whose netbird user row exists and sits lower -------
promoted=0
pending=0
for row in "${want[@]}"; do
  sub="${row%%$'\t'*}"
  rest="${row#*$'\t'}"
  email="${rest%%$'\t'*}"
  target="${row##*$'\t'}"

  current=$(docker exec "$PG_CONTAINER" psql -U "$PG_USER" -d "$NB_DB" -tAc \
    "SELECT role FROM users WHERE id = '${sub}' AND NOT COALESCE(is_service_user, false);" 2>/dev/null | tr -d '[:space:]')

  case "$current" in
    "")
      # No row yet: this person has never completed a dashboard sign-in. Not an
      # error — it is the F-238 constraint. Name them so the gap is visible
      # instead of silent, and pick them up on a later tick.
      pending=$((pending + 1))
      log "PENDING first login: ${email:-$sub} (no netbird user row yet; entitled to ${target})"
      ;;
    owner)
      # Deliberately untouched. See SAFETY rule 1.
      log "owner ${email:-$sub} left as owner (never demoted)"
      ;;
    user|auditor)
      if [ "$(rank "$current")" -ge "$(rank "$target")" ]; then
        : # already at or above the entitled role
      else
        # The WHERE re-states the ladder so a row that changed under us (to
        # owner, say) is never overwritten.
        if [ "$target" = "admin" ]; then
          from_clause="role IN ('user', 'auditor')"
        else
          from_clause="role = 'user'"
        fi
        if docker exec "$PG_CONTAINER" psql -U "$PG_USER" -d "$NB_DB" -q -c \
          "UPDATE users SET role = '${target}' WHERE id = '${sub}' AND ${from_clause} AND NOT COALESCE(is_service_user, false);" >/dev/null 2>&1; then
          promoted=$((promoted + 1))
          log "PROMOTED ${email:-$sub} ${current} -> ${target}"
        else
          log "ERROR promoting ${email:-$sub} ${current} -> ${target} — will retry next tick"
        fi
      fi
      ;;
    admin)
      : # already at the top of the ladder
      ;;
    *)
      log "unexpected role '${current}' for ${email:-$sub} — left alone"
      ;;
  esac
done

log "reconcile complete: ${promoted} promoted, ${pending} awaiting first login, ${n_admin} entitled to admin, ${n_auditor} entitled to auditor, ${stray} stray"

# --- 3. publish the pending count so "awaiting first login" is not log-only --
#
# The `pending` state is the one thing here nobody can fix by deploying: it
# needs the PERSON to sign in once. Leaving it visible only in this host's
# journal is what let two admins sit locked out with
# nobody aware. A node_exporter textfile metric puts it on the same
# Prometheus/Grafana surface the rest of the fleet's health already lands on.
#
# Best-effort by design: no textfile dir (a host without node_exporter) is not
# a reconcile failure, and this script must never fail a tick over telemetry.
if [ -d "$TEXTFILE_DIR" ]; then
  tmp="${TEXTFILE_DIR}/.netbird_admin_reconcile.prom.$$"
  {
    echo "# HELP netbird_admin_reconcile_entitled Kratos identities entitled to the netbird admin role."
    echo "# TYPE netbird_admin_reconcile_entitled gauge"
    echo "netbird_admin_reconcile_entitled ${n_admin}"
    echo "# HELP netbird_admin_reconcile_auditors Kratos identities whose highest mapped netbird role is auditor (read-only dashboard)."
    echo "# TYPE netbird_admin_reconcile_auditors gauge"
    echo "netbird_admin_reconcile_auditors ${n_auditor}"
    echo "# HELP netbird_admin_reconcile_pending_first_login Entitled people with no netbird user row yet. Only THEY can clear this, by signing in to the dashboard once."
    echo "# TYPE netbird_admin_reconcile_pending_first_login gauge"
    echo "netbird_admin_reconcile_pending_first_login ${pending}"
    echo "# HELP netbird_admin_reconcile_promoted_last_run Rows promoted up the user->auditor->admin ladder on the most recent tick."
    echo "# TYPE netbird_admin_reconcile_promoted_last_run gauge"
    echo "netbird_admin_reconcile_promoted_last_run ${promoted}"
    echo "# HELP netbird_admin_reconcile_stray_admins Human netbird admins whose active Kratos identity does not map to admin. This timer never demotes; netbird-role-realign.yml or goat admin platform-admin revoke does."
    echo "# TYPE netbird_admin_reconcile_stray_admins gauge"
    echo "netbird_admin_reconcile_stray_admins ${stray}"
    echo "# HELP netbird_admin_reconcile_last_success_seconds Unix time of the last completed reconcile."
    echo "# TYPE netbird_admin_reconcile_last_success_seconds gauge"
    echo "netbird_admin_reconcile_last_success_seconds $(date +%s)"
  } > "$tmp" 2>/dev/null && mv -f "$tmp" "${TEXTFILE_DIR}/netbird_admin_reconcile.prom" 2>/dev/null || {
    rm -f "$tmp" 2>/dev/null || true
    log "note: could not write ${TEXTFILE_DIR}/netbird_admin_reconcile.prom (telemetry only; reconcile itself succeeded)"
  }
fi
