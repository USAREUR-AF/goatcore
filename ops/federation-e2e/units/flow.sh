#!/usr/bin/env bash
# units/flow.sh — lib/flow.sh measured against a gap we know: a peering up through phase 14
# on role cores (units/pair-role.sh's scenario), a flow from the first side to the second, and
# the second's core router restarted in the middle of it. The flow must see values stored
# before the restart, a bounded run lost while the core's interest returns, and values stored
# again after it. Sourced by lib/unit.sh. Scenario: flow-monitor-measures-a-core-restart.
# shellcheck source=/dev/null
. "$HERE/units/pair-role.sh"
# shellcheck source=/dev/null
. "$HERE/lib/flow.sh"

unit_main() {
  FE2E_CORE=role; export FE2E_CORE
  local n first second r; n="$(role_n)"; first="$(role_first)"; second="$(other "$first")"
  scenario "$n" "$first" "$AVENUE"
  flow_start "$first" "$second" restart
  sleep 15
  rec "fl-$second-core-router-restarted" -- docker exec "$HP-$second-core" docker restart "$second-zenoh-router"
  sleep 45
  flow_stop restart
  r="$(flow_report restart)"
  echo "$r" > "$RECORD/flow-restart.json"
  rec "fl-flow-stored-before-the-restart" -- python3 -c 'import json,sys; r=json.loads(sys.argv[1]); print(r); sys.exit(0 if (r["first_stored"] or 99) <= 5 else 1)' "$r"
  rec "fl-flow-saw-a-bounded-gap" -- python3 -c 'import json,sys; r=json.loads(sys.argv[1]); print(r); sys.exit(0 if 1 <= r["longest_run_lost"] and r["longest_gap_s"] <= 40 else 1)' "$r"
  rec "fl-flow-stored-again-after-it" -- python3 -c 'import json,sys; r=json.loads(sys.argv[1]); print(r); sys.exit(0 if r["last_five_stored"] else 1)' "$r"
}
