#!/usr/bin/env bash
# units/pair-role.sh — the pair unit's scenario (phases 1-14 through one avenue, then the
# failures that make the passes mean something) with each sovereign's core router deployed
# by the ref's own goat-zenoh role (FE2E_CORE=role, lib/engine.sh), then the role's
# committed-slot-list refusal on one of those cores. Sourced by lib/unit.sh.
#
# Avenue goatbuilder: scenario 1, A listens and goes first (steps s1-*).
# Avenue script:      scenario 2, B listens and goes first (steps s2-*).
# Scenarios that read this unit (scenarios/catalog.yaml): core-by-role, preflight-slot-list.

role_n() { [ "$AVENUE" = goatbuilder ] && echo 1 || echo 2; }
role_first() { [ "$AVENUE" = goatbuilder ] && echo "$A" || echo "$B"; }

# The pair unit's checks (persistent reception, the republisher boundary) are reused here.
# shellcheck source=pair.sh
. "$HERE/units/pair.sh"

unit_main() {
  FE2E_CORE=role; export FE2E_CORE
  local n first second; n="$(role_n)"; first="$(role_first)"; second="$(other "$first")"
  scenario "$n" "$first" "$AVENUE"
  role_core_is_the_roles "$n" "$first" "$second"
  role_slot_list "$n" "$first"
  # Last: these stop every responder and probe.
  pair_persistent_reception "$n" "$first" "$second"
  pair_republisher_boundary "$n" "$first" "$second"
}

# The core each side ran phases 1-14 through is the role's: its container name, label,
# compose project and file, the config it mounts, and the digest-pinned image.
role_core_is_the_roles() { # $1 scenario, $2 first, $3 second
  local n="$1" s f
  f='{{.Name}}|{{index .Config.Labels "org.goat.component"}}|{{index .Config.Labels "com.docker.compose.project"}}|{{index .Config.Labels "com.docker.compose.project.config_files"}}|{{range .Mounts}}{{if eq .Destination "/etc/zenoh/config.json5"}}{{.Source}}{{end}}{{end}}|{{.Config.Image}}|{{.State.Running}}'
  for s in "$2" "$3"; do
    rec "s$n-$s-core-is-the-roles" \
      --expect-in "/$s-zenoh-router|zenoh-core|$s-zenoh|/opt/$s-zenoh/docker-compose.yml|/etc/$s-zenoh/config.json5|$ZENOH_IMAGE|true" \
      -- docker exec "$HP-$s-core" docker inspect -f "$f" "$s-zenoh-router"
  done
}

# --- the committed slot list against the live registry -----------------------------------
# The goat-zenoh role refuses to render a committed zenoh_active_slot_anchors that is not the
# live slot set in the portal's registry (tasks/main.yml, "REFUSING to render"), only in
# per-uuid-slot-pool mode with lazy slot storage and a non-empty committed list. The core
# here gets that mode in its group_vars and a registry at the role's default path
# (zenoh_lazy_registry_db, SQLite), the same `slots` rows the portal keeps.

slot_anchor() { printf 'fe2e-slot-%s' "$1" | sha256sum | cut -c1-32; } # a 32-hex anchor

slot_registry() { # $1 sovereign, then anchors: each becomes an assigned, unrevoked slot
  local s="$1"; shift
  host_exec "$HP-$s-core" python3 - "$@" <<'PY'
import os, sqlite3, sys
os.makedirs("/opt/goat-zenoh-portal", exist_ok=True)
c = sqlite3.connect("/opt/goat-zenoh-portal/registry.db")
c.execute("CREATE TABLE IF NOT EXISTS slots (slot_id TEXT PRIMARY KEY, assigned_to TEXT, revoked_at TEXT)")
for a in sys.argv[1:]:
    c.execute("INSERT OR REPLACE INTO slots (slot_id, assigned_to, revoked_at) VALUES (?, ?, NULL)", (a, "member-" + a[:8]))
c.commit()
PY
}

slot_commit() { # $1 sovereign, then the anchors of the committed list
  local s="$1" f="$WORK/$1/slots.yml" a; shift
  {
    echo "# The core router in per-uuid-slot-pool mode with lazy slot storage, and its committed slot list."
    echo "zenoh_identity_model: per-uuid-slot-pool"
    echo "zenoh_lazy_slot_storage: true"
    echo "zenoh_slot_pool_salt: fe2e-slot-salt"
    echo "zenoh_active_slot_anchors:"
    for a in "$@"; do echo "  - \"$a\""; done
  } > "$f"
  core_declare "$s" ops/ansible/group_vars/core_router/slots.yml "$f" "$s: commit the slot list ($# slots)"
}

# The deploy refuses with the role's message and exits non-zero. $1 sovereign, then
# ansible-playbook arguments.
slot_refused() {
  local out rc=0
  out="$(core_deploy "$@" 2>&1)" || rc=$?
  printf '%s\n' "$out" | grep -A3 'REFUSING to render' | head -12
  echo "exit=$rc"
  [ "$rc" -ne 0 ] && printf '%s\n' "$out" | grep -q 'REFUSING to render'
}
# The role's slot-list guard ran and passed. $1 sovereign, then ansible-playbook arguments.
slot_guard_passed() {
  local out
  out="$(core_deploy "$@" 2>&1)" || true
  printf '%s\n' "$out" | grep -A2 'refuse to render an anchor list' | head -6
  printf '%s\n' "$out" | grep -A2 'refuse to render an anchor list' | grep -q '^ok:' \
    && ! printf '%s\n' "$out" | grep -q 'REFUSING to render'
}
# The real run refuses and the running core router was not restarted. $1 sovereign, $2 its
# StartedAt before the run.
slot_refused_no_restart() {
  local s="$1" before="$2" after
  slot_refused "$s" || return 1
  after="$(docker exec "$HP-$s-core" docker inspect -f '{{.State.StartedAt}}' "$s-zenoh-router")"
  echo "started before=$before after=$after"
  [ "$before" = "$after" ]
}
export -f slot_refused slot_guard_passed slot_refused_no_restart

role_slot_list() { # $1 scenario, $2 the sovereign whose core is used
  local n="$1" s="$2" a1 a2 a3 a4 before
  a1="$(slot_anchor 1)"; a2="$(slot_anchor 2)"; a3="$(slot_anchor 3)"; a4="$(slot_anchor 4)"
  # Three live slots, two committed: the core's dry run refuses.
  slot_registry "$s" "$a1" "$a2" "$a3"
  slot_commit "$s" "$a1" "$a2"
  rec "pf-slot-list-refused" -- bash -c "slot_refused $s --check --diff"
  # The list regenerated to the registry: the guard passes the dry run.
  slot_commit "$s" "$a1" "$a2" "$a3"
  rec "pf-slot-list-matching-list-passes-the-dry-run" -- bash -c "slot_guard_passed $s --check --diff"
  # One more slot assigned after that dry run: the run that would restart the router reads
  # the registry again and refuses, and the router keeps running untouched.
  slot_registry "$s" "$a4"
  before="$(docker exec "$HP-$s-core" docker inspect -f '{{.State.StartedAt}}' "$s-zenoh-router")"
  rec "pf-slot-list-refused-on-the-run-that-restarts" -- bash -c "slot_refused_no_restart $s '$before'"
}
