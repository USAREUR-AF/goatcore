#!/usr/bin/env bash
# units/backout-one.sh — B7's two-peering form (docs/design/goatbuilder-v1-gate.md): {A} peers
# with {B} and with {C} on one edge host and one core host, both by goatbuilder; then
# `goatbuilder federate backout {C}` on {A} while values flow from {A} to {B}.
#
# The cores are deployed by the goat-zenoh role (FE2E_CORE=role). The back-out removes
# {A}-{C}'s edge router, tunnel, republisher, core admission and receive store (this topology
# has no forwarder, so there is no forwarder mapping to remove, and the record says so). {A}-{B} is snapshotted before and after: its tunnel, keys, edge and republisher
# configuration and its agreement digest must be unchanged, it passes strict acceptance after,
# and the flow says whether it carried values throughout. Sourced by lib/unit.sh.
# shellcheck source=/dev/null
. "$HERE/units/pair.sh"
# shellcheck source=/dev/null
. "$HERE/units/onecmd.sh"
# shellcheck source=/dev/null
. "$HERE/lib/flow.sh"
OC_PREFIX=bo1

# The back-out (this unit's only oc_run) decides the goat-zenoh role core itself, from the
# core group's site_id, as admission does (admit.rs role_core). The lab's own route to a role
# core, which oc_env names whenever FE2E_CORE=role (a driver-host copy of the org fragment and
# fe2e-core-role), is withheld here as units/nohand.sh withholds it (NH_ADMIT): with a route
# named, the back-out follows it, and the partner's store is not dropped.
BO1_WITHHELD="FED_CORE_ACL_ORG FED_CORE_CERT FED_CORE_KEY FED_CORE_RESTART FED_CORE_DRY_RUN"
oc_env_more() { # $1 us, $2 them
  local out=() i=0 n=${#OC_ENV[@]} name
  while [ "$i" -lt "$n" ]; do
    if [ "${OC_ENV[$i]}" = -e ]; then
      name="${OC_ENV[$((i + 1))]%%=*}"
      case " $BO1_WITHHELD " in *" $name "*) i=$((i + 2)); continue ;; esac
    fi
    out+=("${OC_ENV[$i]}"); i=$((i + 1))
  done
  OC_ENV=("${out[@]}")
}

# {A}'s core admits each peering by goatbuilder's own admission (phase 10), as a deployment
# does, so the back-out finds the admission records it restores from: the core router has one
# leaf for every peering, and backing out the second must put the first one's leaf back. The
# partner CA's review stays the engine's (review_and_admit); only the admission changes. {B}
# and {C} keep the engine's admission (admit_by_role): their back-outs are not measured here.
eval "$(declare -f admit_by_role | sed '1s/^admit_by_role /bo1_admit_by_engine /')"
admit_by_role() { # $1 sovereign, $2 the peering being admitted, $3 its applied dir, $4 scenario number
  local s="$1" them="$2" n="$4" h w="/var/lib/goat-federation/$2/admission"
  if [ "$s" != "$A" ]; then bo1_admit_by_engine "$@"; return; fi
  local d rc=0
  h="$(drv_host "$s")"
  # Admission needs the agreement digest compared with the partner (phase 6); the drive ran
  # phase 6 with --yes, which compares nothing. The digest is read from the partner's copy,
  # as an operator hears it on the call, and passed with --agreement-digest.
  d="$(docker exec "$(drv_host "$them")" sha256sum "/var/lib/goat-federation/$s/agreement.json" | cut -d' ' -f1)"
  oc_run "$s" "$them" "confirm-$them" "$them" --from 6 --to 6 --agreement-digest "$d" || rc=$?
  # Phase 10 at a terminal, approved as an operator approves it (lib/answer.py).
  [ "$rc" != 0 ] || oc_run "$s" "$them" "admit-$them" "$them" --from 10 --to 10 || rc=$?
  # Admitted means both runs passed: a refused admission still leaves its run folder in $w.
  rec "s$n-$s-$them-admitted-by-goatbuilder" -- bash -c \
    "sed 's/\x1b\[[0-9;]*m//g' '$RECORD/$OC_PREFIX-$s-admit-$them.log' 2>/dev/null | tr -d '\r' | tail -15; docker exec $h ls -l $w; exit $rc"
}

# What {A} holds of its peering with $1, on the edge and on the core.
bo1_holds() { # $1 them -> stdout, one line per resource: "<what> present|absent"
  local them="$1" i
  i="$(iface "$A" "$them")"
  docker exec "$HP-$A-edge" sh -c "
    docker inspect goat-zenoh-edge-$them >/dev/null 2>&1 && echo 'edge router container present' || echo 'edge router container absent'
    ip link show $i >/dev/null 2>&1 && echo 'tunnel $i present' || echo 'tunnel $i absent'
    [ -e /etc/goat-zenoh-edge-$them ] && echo 'edge router configuration present' || echo 'edge router configuration absent'"
  docker exec "$HP-$A-core" sh -c "
    docker inspect goat-fed-republisher-$them >/dev/null 2>&1 && echo 'republisher container present' || echo 'republisher container absent'
    [ -e /etc/goat-fed-republisher-$them ] && echo 'republisher configuration present' || echo 'republisher configuration absent'
    grep -q 'sovereign-$them-' /etc/*-zenoh/config.json5 2>/dev/null && echo 'core admission present' || echo 'core admission absent'
    grep -q 'sovereign_from_$them' /etc/*-zenoh/config.json5 2>/dev/null && echo 'core receive store present' || echo 'core receive store absent'"
}

# The lab reads each core's REST surface from the docker host, at <core>:8000; the engine's
# own role runs bind it there with -e goat_zenoh_rest_host=0.0.0.0. goatbuilder's admission
# and back-out run zenoh-deploy.yml, whose play pins the bind to 127.0.0.1 (play vars outrank
# any deployment value), so after them {A}'s REST answers on the core host only. A relay on
# the core host, lab-side and outside the deployment, carries <core>:8000 to 127.0.0.1:8000
# for the whole run (binding once the engine's own 0.0.0.0 bind is gone); it survives the
# role's re-runs, which rebind 127.0.0.1 only. LAB ONLY: it exposes the core's admin REST
# surface on the lab's own network solely for the harness's readers; no deployment does this,
# and the play's 127.0.0.1 pin is left as it is.
bo1_rest_relay() {
  local h="$HP-$A-core" ip; ip="$(core_ip "$A")"
  host_exec "$h" sh -c 'cat > /usr/local/bin/fe2e-rest-relay.py' <<'PY'
import socket, sys, threading
listen, target = sys.argv[1], ("127.0.0.1", int(sys.argv[2]))
def pipe(a, b):
    try:
        while True:
            d = a.recv(65536)
            if not d:
                break
            b.sendall(d)
    except OSError:
        pass
    finally:
        for s in (a, b):
            try:
                s.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
import time
# The engine's first deploy binds 0.0.0.0:8000 itself; the address frees once goatbuilder's
# role run rebinds 127.0.0.1, so the bind is retried until then.
while True:
    srv = socket.socket(); srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        srv.bind((listen, target[1])); break
    except OSError:
        srv.close(); time.sleep(1)
srv.listen(64)
while True:
    c, _ = srv.accept()
    try:
        u = socket.create_connection(target, timeout=10)
    except OSError:
        c.close(); continue
    u.settimeout(None)
    threading.Thread(target=pipe, args=(c, u), daemon=True).start()
    threading.Thread(target=pipe, args=(u, c), daemon=True).start()
PY
  docker exec -d "$h" python3 /usr/local/bin/fe2e-rest-relay.py "$ip" 8000
}

unit_main() {
  # The back-out drops the partner's store by re-running the core's goat-zenoh role, so the
  # core routers are the role's (as units/nohand.sh and units/pair-role.sh deploy them).
  FE2E_CORE=role; export FE2E_CORE
  local n=5 rc=0
  set_partners "$A" "$B $C"; set_partners "$B" "$A"; set_partners "$C" "$A"; export NODIR="" PERCP=""
  # {A} declares its peering with {C} only when it starts (declare_later), as a deployment
  # adds a second peering: declared from the start, it would have no agreement while {A}-{B}
  # is admitted (host-allocation reads SKIP, GB-188), or its tunnel would already be on the
  # shared edge when {A}-{B}'s phase 11 loads the host firewall baseline (GB-210).
  printf -v "FE2E_LATER_$A" '%s' "$C"; export "FE2E_LATER_$A"
  export_helpers
  say "scenario $n: ${A} federates with ${B} and with ${C} on one edge host and one core host, both by goatbuilder"
  setup_hosts "$n"
  # {A}'s core group's site_id, as a deployment declares it for its goat-zenoh core
  # (units/nohand.sh the same): admission and the back-out both find the role core from it.
  printf '%s\n' "# $A's core router site (the goat-zenoh role's mandatory site_id)." "site_id: $A" > "$WORK/$A/core-site.yml"
  core_declare "$A" ops/ansible/group_vars/core_router/site.yml "$WORK/$A/core-site.yml" "$A: the core router's site"
  bo1_rest_relay
  peer_up "$n" "$A" "$B" goatbuilder
  declare_later "$A" "$C"
  peer_up "$n" "$A" "$C" goatbuilder
  accept_pair "$n" "$A" "$B" goatbuilder "-before-backout"
  accept_pair "$n" "$A" "$C" goatbuilder "-before-backout"

  # {A}-{B} as it stands before the back-out, and what {A} holds of {A}-{C}.
  snap "$A" "$B" "$C" > "$RECORD/bo1-$B-before.txt"
  docker exec "$HP-$A-edge" sha256sum "/var/lib/goat-federation/$B/agreement.json" | cut -d' ' -f1 > "$WORK/bo1-$A-$B-agreement.sha"
  bo1_holds "$C" > "$RECORD/bo1-$C-held-before.txt"
  rec "bo1-$A-holds-$C-before" -- bash -c "cat '$RECORD/bo1-$C-held-before.txt'; ! grep -q ' absent$' '$RECORD/bo1-$C-held-before.txt'"

  say "scenario $n: back out ${A}-${C} while values flow from ${A} to ${B}"
  flow_start "$A" "$B" bo1
  oc_run "$A" "$C" backout backout "$C" --yes || rc=$?
  # The flow keeps going a little past the command's end, so a restart late in the back-out
  # is inside the window it measures.
  sleep 20
  flow_stop bo1
  flow_report bo1 > "$RECORD/bo1-flow.json"
  docker cp -q "$(drv_host "$A"):/var/lib/goat-federation/$C/backout" "$RECORD/bo1-$A-backout-record" 2>/dev/null || true
  # The command's run record (its checks, each with why it passed or not), the newest one.
  local rr
  rr="$(docker exec "$(drv_host "$A")" sh -c 'ls -td /root/.goat/state/runs/*-federate-* 2>/dev/null | head -1')"
  [ -n "$rr" ] && docker cp -q "$(drv_host "$A"):$rr" "$RECORD/bo1-$A-backout-run" 2>/dev/null || true
  # A back-out that stopped: the same back-out as a plan with -o json, which prints every row
  # the table view leaves out, the stop's reason and code among them. It changes nothing.
  if [ "$rc" != 0 ]; then
    oc_env "$A" "$C"
    docker exec "${OC_ENV[@]}" -w /opt/goat "$(drv_host "$A")" goatbuilder -C /opt/goat -o json federate backout "$C" --dry-run \
      > "$RECORD/bo1-$A-backout-rows.json" 2> "$RECORD/bo1-$A-backout-rows.err" || true
  fi
  rec "bo1-$A-$C-backout-ran" -- bash -c \
    "sed 's/\x1b\[[0-9;]*m//g' '$RECORD/$OC_PREFIX-$A-backout.log' | tr -d '\r' | tail -40; exit $rc"

  # {A}-{C}: everything the command created for it is gone.
  bo1_holds "$C" > "$RECORD/bo1-$C-held-after.txt"
  rec "bo1-$A-$C-removed" -- bash -c "cat '$RECORD/bo1-$C-held-after.txt'; echo 'forwarder mapping: not applicable (no forwarder in this topology)'; ! grep -q ' present$' '$RECORD/bo1-$C-held-after.txt'"

  # {A}-{B}: nothing of its own changed, the same agreement, and it carried values throughout.
  snap "$A" "$B" "$C" > "$RECORD/bo1-$B-after.txt"
  rec "bo-other-peering-unchanged" -- bash -c \
    "diff '$RECORD/bo1-$B-before.txt' '$RECORD/bo1-$B-after.txt' && a=\$(docker exec $HP-$A-edge sha256sum /var/lib/goat-federation/$B/agreement.json | cut -d' ' -f1) && echo \"agreement \$a, before \$(cat '$WORK/bo1-$A-$B-agreement.sha')\" && [ \"\$a\" = \"\$(cat '$WORK/bo1-$A-$B-agreement.sha')\" ]"
  rec "bo1-$A-$B-carried-throughout" -- python3 -c "
import json, sys
f = json.load(open('$RECORD/bo1-flow.json')); print(json.dumps(f))
sys.exit(0 if f['released'] > 0 and f['lost'] == 0 else 1)"
  accept_pair "$n" "$A" "$B" goatbuilder "-after-backout"
}

# The relay is the lab's, for this run only. It is stopped on the unit's exit path, which
# every end of the run takes (a die, the watchdog, a signal), before the engine's own final
# step, so a lab kept after a failed run (KEEP=1) holds no listener the deployment never had.
eval "$(declare -f final | sed '1s/^final /bo1_engine_final /')"
final() {
  docker exec "$HP-$A-core" pkill -f fe2e-rest-relay.py >/dev/null 2>&1 || true
  bo1_engine_final
}
