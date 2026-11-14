#!/usr/bin/env bash
# units/workspace.sh — two sovereigns, each driven from its operator's own workspace: a
# third host per sovereign ($HP-<s>-ops) on the sovereign's internal network only, as an
# operator's laptop reaches only its own sovereign's hosts. The workspace holds the clone
# (/opt/goat), goat and goatbuilder, the goat context, the working directory and the ssh key
# that reaches its own edge and core; the inventory names the edge and the core by their
# addresses (overlay.py --edge-addr). Phases 1-14 for the pair from both workspaces, with
# phase 3's package checked for what only the edge and the core can measure. Sourced by
# lib/unit.sh. The scenario reads: operator-workspace.
#
# The workspace reaches no boundary network, so it reaches no peering directory either: the
# two operators exchange their packages by hand (FED_NO_DIRECTORY=1), as the runbook allows
# for a counterparty that can reach none.
#
# Phase 3 reaches the core router through the variable the driver documents for it,
# FED_ROUTER_DOCKER_HOST=ssh://goat@<core> (drive), and the edge through the inventory's
# address, user and key, which the plays already use. Nothing else is passed: a ref whose
# phase 3 measures the machine it runs on fails phase 3 here, and the reason is the
# driver's own output.
FE2E_DRIVER=workspace; export FE2E_DRIVER

# The package one side wrote in phase 3, on its workspace. $1 us, $2 them, $3 python
# expression over p (the package) that prints the value.
ws_package() {
  docker exec "$(drv_host "$1")" cat "/var/lib/goat-federation/$2/ours.canonical.json" 2>/dev/null \
    | python3 -c "import json,sys; p=json.load(sys.stdin); print($3)"
}
export -f ws_package

# Before the exchange: each side's package is checked against the hosts it describes. When
# either side has no package, phase 3 did not write one and nothing later can run: the unit
# stops there, naming the phase that failed and the driver's last words for it.
eval "ws_exchange_inner() $(declare -f exchange | tail -n +2)"
exchange() { # the engine's exchange, after the package checks
  local first="$1" second="$2" n="$4" s them missing="" why="" log
  for s in "$first" "$second"; do
    them="$(other "$s")"
    rec "s$n-$s-package-written-on-the-workspace" -- docker exec "$(drv_host "$s")" test -s "/var/lib/goat-federation/$them/ours.canonical.json"
    docker exec "$(drv_host "$s")" test -s "/var/lib/goat-federation/$them/ours.canonical.json" || missing="$missing $s"
    # The interface key exists only on the edge: the workspace has no interface of that name.
    rec "s$n-$s-package-carries-the-edge-interface-key" -- bash -c \
      "k=\$(docker exec $HP-$s-edge wg show $(iface "$s" "$them") public-key); p=\$(ws_package $s $them 'p[\"transport_offer\"][\"public_key\"]'); echo \"edge=\$k package=\$p\"; [ -n \"\$k\" ] && [ \"\$k\" = \"\$p\" ]"
    # The fabric version comes from the core router's own zenohd (/zenohd in the fleet image).
    rec "s$n-$s-package-carries-the-core-router-version" -- bash -c \
      "v=\$(docker exec $HP-$s-core docker exec $s-zenoh-router /zenohd --version 2>&1 | head -1); p=\$(ws_package $s $them 'p[\"capabilities\"][\"fabric_version\"]'); echo \"router='\$v' package=\$p\"; [ -n \"\$p\" ] && case \"\$v\" in *\"\$p\"*) exit 0 ;; esac; exit 1"
  done
  if [ -n "$missing" ]; then
    for s in $missing; do
      log="$(ls "$RECORD"/[0-9]*"-s$n-$s-$(other "$s")-phases-1-to-5.log" 2>/dev/null | head -1)"
      # The phase that failed with its exit code, then each value init could not derive.
      why="$why $s: $(grep -oE 'Phase [0-9]+ \("[^"]*"\) FAILED . exit [0-9]+' "$log" 2>/dev/null | head -1);$(grep -E '^\[.\] [a-z_.]+ . cannot derive|GB-[0-9]+' "$log" 2>/dev/null | grep -v -e '^\[.\] package ' -e '!!' | head -4 | sed 's/^/ /' | tr '\n' ';')"
    done
    die "phase 3 wrote no package on the workspace of$missing; nothing after the exchange can run.$why"
  fi
  ws_exchange_inner "$@"
}

ws_isolation() { # $1 sovereign: what its workspace reaches, and what it must not
  local s="$1" t; t="$(other "$s")"
  local ssh="ssh -o BatchMode=yes -o ConnectTimeout=5"
  rec "ws-$s-workspace-reaches-its-edge" --expect-in "$HP-$s-edge" -- docker exec "$HP-$s-ops" $ssh "goat@$(edge_ip "$s")" hostname
  rec "ws-$s-workspace-reaches-its-core" --expect-in "$HP-$s-core" -- docker exec "$HP-$s-ops" $ssh "goat@$(core_ip "$s")" hostname
  rec "ws-$s-workspace-cannot-reach-$t-edge" --expect-exit 1 -- docker exec "$HP-$s-ops" ping -c1 -W2 "$(edge_ip "$t")"
  rec "ws-$s-workspace-cannot-reach-$t-core" --expect-exit 1 -- docker exec "$HP-$s-ops" ping -c1 -W2 "$(core_ip "$t")"
  rec "ws-$s-workspace-cannot-reach-$t-boundary" --expect-exit 1 -- docker exec "$HP-$s-ops" ping -c1 -W2 "$(boundary_ip "$t")"
  rec "ws-$s-workspace-cannot-reach-its-own-boundary" --expect-exit 1 -- docker exec "$HP-$s-ops" ping -c1 -W2 "$(boundary_ip "$s")"
  rec "ws-$s-workspace-cannot-reach-the-directory" --expect-exit 1 -- docker exec "$HP-$s-ops" ping -c1 -W2 "$DIRECTORY_IP"
  rec "ws-$s-workspace-holds-no-boundary-address" --expect-exit 1 -- docker exec "$HP-$s-ops" sh -c "ip -o -4 addr | grep -F ' $NB.3.'"
}

unit_main() {
  local n=ws s
  [ "$AVENUE" = goatbuilder ] || die "the operator-workspace unit drives goatbuilder only, not $AVENUE"
  set_partners "$A" "$B"; set_partners "$B" "$A"; set_partners "$C" ""
  # shellcheck disable=SC2034 # NODIR is read by lib/engine.sh (nodir)
  NODIR="$A:$B $B:$A"
  export_helpers
  setup_hosts "$n"
  say "operator workspaces: each reaches its own edge and core, and nothing else"
  for s in "$A" "$B"; do ws_isolation "$s"; done
  # The operator's goat context lives on the workspace only.
  for s in "$A" "$B"; do
    rec "ws-$s-edge-holds-no-goat-context" --expect-exit 1 -- docker exec "$HP-$s-edge" test -e /root/.goat/contexts
  done
  peer_up "$n" "$A" "$B" goatbuilder
  collect "$n"
}
