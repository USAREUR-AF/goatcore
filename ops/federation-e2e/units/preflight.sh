#!/usr/bin/env bash
# units/preflight.sh — the refusals a federation owes before it changes anything, and the
# network profile of an edge whose UDP 51820 is already taken. Two parties, {A} listens.
# Sourced by lib/unit.sh. Three fresh sets of hosts, one after another, in one lab:
#
#   1. port (steps pf-port-*, s-pf-port-*): another process holds UDP 51820 on {A}'s edge
#      before phase 1. Phase 1 must refuse, naming the port and a fix. When it does not,
#      phases 1-12 run on and what they do with the held port is recorded. On the same
#      hosts, {B}'s phase 3 (`goat federate init` through FED_ROUTER_DOCKER_HOST) is run
#      against its core router under the standard, legacy and a custom name, and with two
#      routers running (steps pf-router-*).
#   2. held (steps pf-held-*, s-pf-held-*): the same held port, and {A}'s peering on the
#      free inner port 51823 (peering_wg_listen_port, and the endpoint its package offers).
#      The pair must handshake and pass strict acceptance.
#   3. wider (steps pf-wider-*, s-pf-wider-*): {A}'s core receive store for {B} widened to
#      from/** where the edge grants from/{B}/<set>/**. Some phase before 14 must refuse,
#      naming the store and the grant.
#
# Scenarios that read this unit (scenarios/catalog.yaml): preflight-port-in-use,
# port-51820-held, preflight-router-discovery, preflight-store-wider.

PF_HELD_PORT=51820
PF_INNER_PORT=51823

# Whether the last recorded step of this name passed. $1 step name.
pf_passed() {
  python3 - "$RECORD/report.jsonl" "$1" <<'PY'
import json, sys
st = None
try:
    for line in open(sys.argv[1]):
        r = json.loads(line)
        if r["name"] == sys.argv[2]:
            st = r["status"]
except FileNotFoundError:
    pass
sys.exit(0 if st == "pass" else 1)
PY
}

# Another service holding UDP $PF_HELD_PORT on an edge host, as NetBird does on a real
# edge: a transient systemd unit inside the host, so that it outlives this shell.
pf_hold_port() { # $1 sovereign
  docker exec "$HP-$1-edge" systemd-run --unit=pf-hold-udp-$PF_HELD_PORT --collect \
    python3 -c "import socket, time; s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind(('0.0.0.0', $PF_HELD_PORT)); time.sleep(86400)" >/dev/null
  sleep 1
}

# The core router recreated on a core host's own dockerd, with start_core's arguments, under
# a name of our choosing and with or without the role's label. Every router this unit may
# have started there is removed first. $1 sovereign, $2 name, $3 label|nolabel.
pf_router() {
  local s="$1" name="$2" label="$3" h="$HP-$1-core" lab=()
  host_exec "$h" sh -c "docker rm -f $s-zenoh-router goat-zenoh $s-fabric-core $s-fabric-labelled $s-zenoh-spare >/dev/null 2>&1; true"
  [ "$label" = label ] && lab=(--label org.goat.component=zenoh-core)
  host_exec "$h" docker run -d --name "$name" ${lab[@]+"${lab[@]}"} --restart unless-stopped \
    -e RUST_LOG=zenoh=info,zenoh::net::routing::interceptor::access_control=trace \
    -p 7447:7447 -p 8000:8000 -v /etc/goat-core:/etc/goat-core:ro \
    "$ZENOH_IMAGE" -c /etc/goat-core/config.json5 >/dev/null
}
# A second core router beside the first: same image and configuration, the role's label,
# no published port (it has its own network namespace on the core host).
pf_spare_router() { # $1 sovereign
  host_exec "$HP-$1-core" docker run -d --name "$1-zenoh-spare" --label org.goat.component=zenoh-core \
    -v /etc/goat-core:/etc/goat-core:ro "$ZENOH_IMAGE" -c /etc/goat-core/config.json5 >/dev/null
}
pf_router_id() { host_exec "$HP-$1-core" docker inspect -f '{{.Id}}' "$2"; } # $1 sovereign, $2 container

# drive, with more environment for the driver on the edge (FED_ROUTER_CONTAINER). $1 the
# extra "K=V ..." list, then drive's own arguments. Runs in a `rec ... bash -c` child.
drive_with() {
  local extra="$1"; shift
  docker() {
    if [ "$1" = exec ]; then
      shift; local kv e=(); for kv in $extra; do e+=(-e "$kv"); done
      command docker exec ${e[@]+"${e[@]}"} "$@"
    else command docker "$@"; fi
  }
  local rc=0; drive "$@" || rc=$?
  unset -f docker
  return "$rc"
}
# Phase 3 on one side, from a clean slate: our package removed first, so that the file
# afterwards is this run's. Prints the driver's output; exit 0 only when phase 3 exited 0
# AND the package it wrote carries the router's measured fabric version.
pf_phase3() { # $1 sovereign, $2 them, $3 avenue, [$4 extra environment]
  local s="$1" them="$2" av="$3" extra="${4:-}" rc
  docker exec "$HP-$s-edge" rm -f "/var/lib/goat-federation/$them/ours.canonical.json"
  drive_with "$extra" "$s" "$them" "$av" 3 3; rc=$?
  echo "phase 3 exit=$rc"
  [ "$rc" = 0 ] || return "$rc"
  docker exec "$HP-$s-edge" grep -o '"fabric_version":"[^"]*"' "/var/lib/goat-federation/$them/ours.canonical.json" || {
    echo "phase 3 exited 0 but the package names no fabric_version"; return 1; }
}

# Phases 1-12 of one peering, both sides, as peer_up runs them, stopping at the first gap
# instead of dying: what a held port does to the phases after a phase 1 that let it through.
pf_up_to_12() { # $1 scenario label, $2 listener, $3 dialer, $4 avenue
  local n="$1" first="$2" second="$3" av="$4" s them
  prepare_edge "$first" "$second" true
  prepare_edge "$second" "$first" false
  for s in "$first" "$second"; do
    them="$(other "$s")"
    rec "s$n-$s-$them-phases-1-to-5" --expect-in "exchange through the directory" -- bash -c "drive '$s' '$them' '$av' 1 5"
  done
  pf_passed "s$n-$first-$second-phases-1-to-5" && pf_passed "s$n-$second-$first-phases-1-to-5" || return 0
  exchange "$first" "$second" "$av" "$n"
  for s in "$first" "$second"; do
    them="$(other "$s")"
    rec "s$n-$s-$them-phases-6-to-10" --expect-in "review what apply rendered" -- bash -c "drive '$s' '$them' '$av' 6 10"
  done
  pf_passed "s$n-$first-$second-phases-6-to-10" && pf_passed "s$n-$second-$first-phases-6-to-10" || return 0
  review_and_admit "$first" "$second"
  review_and_admit "$second" "$first"
  for s in "$first" "$second"; do
    them="$(other "$s")"
    rec "s$n-$s-$them-phases-11-to-12" --expect-in "open the wireguard transport" -- bash -c "drive '$s' '$them' '$av' 11 12"
  done
  rec "s$n-$first-$second-a-wireguard-handshake" -- bash -c \
    "for _ in \$(seq 1 15); do t=\$(docker exec $HP-$first-edge wg show $(iface "$first" "$second") latest-handshakes 2>/dev/null | cut -f2); [ \"\${t:-0}\" != 0 ] && break; sleep 2; done; echo \"latest handshake: \${t:-none}\"; [ \"\${t:-0}\" != 0 ]"
}

# Summarize the recorded phase steps whose names match a pattern: which failed, and the
# lines of their logs that match a second pattern. Exit 0 when a failed one names it.
pf_failed_naming() { # $1 step-name regex, $2 log-line regex (extended), [$3 a second regex that must also appear]
  python3 - "$RECORD" "$1" "$2" "${3:-}" <<'PY'
import json, os, re, sys
rec, pat, want, also = sys.argv[1], re.compile(sys.argv[2]), re.compile(sys.argv[3]), sys.argv[4]
hit = False
for line in open(os.path.join(rec, "report.jsonl")):
    r = json.loads(line)
    if not pat.fullmatch(r["name"]):
        continue
    text = open(os.path.join(rec, r["log"]), errors="replace").read()
    lines = [l.strip() for l in text.splitlines() if want.search(l)]
    print(f"{r['name']}: {r['status']} (exit {r['exit']}); {len(lines)} line(s) match")
    for l in lines[:6]:
        print("    " + l[:400])
    if r["status"] != "pass" and lines and (not also or re.search(also, text)):
        hit = True
sys.exit(0 if hit else 1)
PY
}

# --- 1. the port held before phase 1, and router discovery -----------------------------
pf_port_and_router() {
  local n="-pf-port" first="$A" second="$B" av="$AVENUE" out
  setup_hosts "$n"
  prepare_edge "$first" "$second" true
  prepare_edge "$second" "$first" false
  pf_hold_port "$first"
  rec "pf-port-held-before-phase-1" --expect-in ":$PF_HELD_PORT" -- docker exec "$HP-$first-edge" ss -Hulpn "sport = :$PF_HELD_PORT"
  out="$RECORD/pf-port-phase-1.out"
  rec "pf-port-in-use-refused" -- bash -c \
    "drive '$first' '$second' '$av' 1 1 > '$out' 2>&1; rc=\$?; cat '$out'; echo \"phase 1 exit=\$rc\"
     [ \$rc -ne 0 ] || { echo 'phase 1 did not refuse with UDP $PF_HELD_PORT held on the edge'; exit 1; }
     grep -q '$PF_HELD_PORT' '$out' || { echo 'the refusal does not name port $PF_HELD_PORT'; exit 1; }
     grep -qiE 'peering_wg_listen_port|inner port|free port|another port|forwarder' '$out' || { echo 'the refusal names no fix'; exit 1; }"
  rec "pf-port-in-use-no-change-before-refusal" -- docker exec "$HP-$first-edge" sh -c \
    "if ip link show $(iface "$first" "$second") >/dev/null 2>&1 || [ -e /etc/goat-peering-wg ]; then echo 'phase 1 left the interface or its config behind'; exit 1; fi; echo 'no interface, no /etc/goat-peering-wg'"

  # Router discovery: phase 3 on the dialing side, whose port is free.
  rec "pf-router-phases-1-to-2" -- bash -c "drive '$second' '$first' '$av' 1 2"
  if pf_passed "pf-router-phases-1-to-2"; then
    pf_router "$second" "$second-zenoh-router" label
    rec "pf-router-standard-name" -- bash -c "pf_phase3 '$second' '$first' '$av'"
    pf_router "$second" "goat-zenoh" nolabel
    rec "pf-router-legacy-name" -- bash -c "pf_phase3 '$second' '$first' '$av'"
    pf_router "$second" "$second-fabric-labelled" label
    rec "pf-router-custom-name-with-label" -- bash -c "pf_phase3 '$second' '$first' '$av'"
    pf_router "$second" "$second-fabric-core" nolabel
    rec "pf-router-custom-name-refused" --expect-in "FED_ROUTER_CONTAINER" -- bash -c \
      "if pf_phase3 '$second' '$first' '$av'; then echo 'phase 3 found a custom-named, unlabelled router without FED_ROUTER_CONTAINER'; exit 1; fi; exit 0"
    rec "pf-router-custom-name-with-variable" -- bash -c "pf_phase3 '$second' '$first' '$av' FED_ROUTER_CONTAINER=$second-fabric-core"
    pf_router "$second" "$second-zenoh-router" label
    pf_spare_router "$second"
    local id1 id2; id1="$(pf_router_id "$second" "$second-zenoh-router")"; id2="$(pf_router_id "$second" "$second-zenoh-spare")"
    rec "pf-router-several-refused" --expect-in "$id1" --expect-in "$id2" --expect-in "FED_ROUTER_CONTAINER" -- bash -c \
      "if pf_phase3 '$second' '$first' '$av'; then echo 'phase 3 chose one of two running routers'; exit 1; fi; exit 0"
    pf_router "$second" "$second-zenoh-router" label
  else
    for c in standard legacy custom-with-label custom-refused custom-with-variable several-refused; do
      mark "preflight-router-discovery.$c" BLOCKED "phases 1-2 on $second did not complete, so phase 3 could not be reached"
    done
  fi

  # Today's behavior when phase 1 lets the held port through.
  if pf_passed "pf-port-in-use-refused"; then
    mark "preflight-port-in-use.later-phase-names-port" "NOT APPLICABLE" "phase 1 refused, so no later phase met the held port"
  else
    pf_up_to_12 "$n" "$first" "$second" "$av"
    # The driver's own output is what the operator reads; the tunnel service's journal on
    # the edge is printed after it as the cause, and does not count as naming the port.
    rec "pf-port-later-phase-names-port" -- bash -c \
      "pf_failed_naming 's$n-$first-$second-(phases-.*|phase-13)' '$PF_HELD_PORT|[Aa]ddress already in use|EADDRINUSE'; rc=\$?
       echo '--- the tunnel service on the edge (not shown by the driver):'
       docker exec $HP-$first-edge journalctl --no-pager -u 'goat-peering-wg@$(iface "$first" "$second")' 2>&1 | tail -8
       exit \$rc"
  fi
  collect "$n"
}

# --- 2. the same held port, the peering on a free inner port -----------------------------
pf_held_inner_port() {
  local n="-pf-held" first="$A" second="$B" av="$AVENUE" c
  setup_hosts "$n"
  # The listener's declarations: its tunnel listens on the inner port, and the endpoint
  # its package offers (federation.toml, through wgport) is that port.
  c="$WORK/$first/clone"
  printf 'peering_wg_listen_port: %s\n' "$PF_INNER_PORT" >> "$c/ops/ansible/group_vars/${second}_edge/deployment.yml"
  git -C "$c" commit -q -am "$first: the peering with $second listens on $PF_INNER_PORT, 51820 is taken"
  git -C "$c" push -q origin main
  host_exec "$HP-$first-edge" git -C /opt/goat pull -q --ff-only origin main
  pf_hold_port "$first"
  rec "pf-held-port-held-before-phase-1" --expect-in ":$PF_HELD_PORT" -- docker exec "$HP-$first-edge" ss -Hulpn "sport = :$PF_HELD_PORT"
  peer_up "$n" "$first" "$second" "$av"
  rec "pf-held-listens-on-the-inner-port" --expect-in "$PF_INNER_PORT" -- docker exec "$HP-$first-edge" wg show "$(iface "$first" "$second")" listen-port
  rec "pf-held-dialer-endpoint-is-the-inner-port" --expect-in "$(boundary_ip "$first"):$PF_INNER_PORT" -- \
    docker exec "$HP-$second-edge" wg show "$(iface "$second" "$first")" endpoints
  rec "pf-held-port-still-held" --expect-in "python3" -- docker exec "$HP-$first-edge" ss -Hulpn "sport = :$PF_HELD_PORT"
  collect "$n"
}

# --- 3. a core receive store wider than the edge grants ----------------------------------
pf_store_wider() {
  local n="-pf-wider" first="$A" second="$B" av="$AVENUE"
  setup_hosts "$n"
  # The receiving core (first) stores everything under from/** instead of the exact
  # landing for its partner. Phase 10 re-renders the core from this seed.
  sed -i "s|landing: { key_expr: \"from/$second/\\*\\*\"|landing: { key_expr: \"from/**\"|" "$WORK/$first/core-seed.json5"
  cp "$WORK/$first/core-seed.json5" "$WORK/$first/core.json5"
  docker cp -q "$WORK/$first/core.json5" "$HP-$first-core:/etc/goat-core/config.json5"
  host_exec "$HP-$first-core" docker restart "$first-zenoh-router" >/dev/null
  rec "pf-wider-store-is-from-all" --expect-in 'key_expr: "from/**"' --expect-not-in "key_expr: \"from/$second/**\"" -- \
    docker exec "$HP-$first-core" cat /etc/goat-core/config.json5
  peer_up "$n" "$first" "$second" "$av"
  rec "pf-wider-store-still-from-all-after-phase-10" --expect-in 'key_expr: "from/**"' -- docker exec "$HP-$first-core" cat /etc/goat-core/config.json5
  # The refusal: a phase before 14, either side, failed naming the store and the grant.
  rec "pf-store-wider-refused" -- bash -c \
    "pf_failed_naming 's$n-($first-$second|$second-$first)-(phases-.*|phase-13)' 'from/\\*\\*' 'from/$second/'"
  # What the edge's own ingress-storage inspection said during phase 12.
  rec "pf-wider-edge-inspection-said" --expect-in "PASS: a matching receive store" -- bash -c \
    "grep -hE 'ingress storage|receive store|SKIP: the local core|Missing exact scopes' '$RECORD'/*-s$n-$first-$second-phases-11-to-12.log | head -10"
  collect "$n"
}

unit_main() {
  set_partners "$A" "$B"; set_partners "$B" "$A"; export NODIR=""
  # The held-port profile offers the inner port as the listener's endpoint.
  wgport() {
    if [ "${PF_INNER_FOR:-}" = "$1:$2" ]; then echo "$PF_INNER_PORT"; else echo $((51820 + $(pidx "$1" "$2"))); fi
  }
  export PF_HELD_PORT PF_INNER_PORT
  export -f pf_passed pf_phase3 drive_with pf_failed_naming
  export_helpers
  pf_port_and_router
  export PF_INNER_FOR="$A:$B"
  pf_held_inner_port
  export PF_INNER_FOR=""
  pf_store_wider
}
