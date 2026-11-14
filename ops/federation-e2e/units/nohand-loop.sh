# shellcheck shell=bash
# nothing-hand-written as a fix-and-resume loop (FE2E_NH_LOOP=1, sourced by units/nohand.sh).
#
# The lab stays up. At the first refusal of a run of the command (or of acceptance) the unit
# writes one line to $FE2E_LOOP_NOTES and to $WORK/loop/state, and waits. Nothing is given
# back and nothing is admitted by hand. The fixer then writes $WORK/loop/go with one line:
#   <side> <phase>
# and the unit installs the fixer's build and tree and resumes that side with --from <phase>:
#   FE2E_LOOP_BIN   a directory holding goatbuilder (and goat) built from the fixer's tree;
#                   copied over the binaries the lab's hosts mount, so a resume uses them
#   FE2E_LOOP_TREE  the fixer's working tree; its changes outside crates/ since the ref the lab
#                   started from are applied to each sovereign's clone and its driver host.
# Phase 0 resumes with the plain run (no --from).

LOOP="$WORK/loop"; mkdir -p "$LOOP"
# One line per refusal and per resume; the record keeps a copy (loop.md) in every mode.
FE2E_LOOP_NOTES="${FE2E_LOOP_NOTES:-$RECORD/loop.md}"
# The binaries the hosts mount are this run's own copies, so replacing their content
# (same inode) reaches the running hosts.
cp "$GB" "$LOOP/goatbuilder"; GB="$LOOP/goatbuilder"
cp "$GOAT" "$LOOP/goat"; GOAT="$LOOP/goat"
export GB GOAT
LOOP_N=0

loop_note() {
  local l; l="$(date -u +%H:%M:%SZ) $*"
  printf '%s\n' "$l" >> "$LOOP/state"
  [ "$FE2E_LOOP_NOTES" = "$RECORD/loop.md" ] || printf '%s\n' "$l" >> "$RECORD/loop.md"
  printf '%s\n' "$l" >> "$FE2E_LOOP_NOTES"
}

# The refusal of a run: the first failed line of the transcript, cleaned.
loop_refusal() { # $1 transcript
  sed 's/\x1b\[[0-9;]*m//g; s/\r//g' "$1" 2>/dev/null \
    | grep -m3 -E '✗|FAIL |GB-[0-9]+|refused|ACCEPTANCE FAILED|ACCEPTANCE INCOMPLETE|BLOCKED' | sed 's/^ *//' | tr '\n' ' ' | cut -c1-400
}

loop_install() { # the fixer's build and tree onto the lab
  local s f b
  if [ -n "${FE2E_LOOP_BIN:-}" ]; then
    for b in goatbuilder goat; do
      f="$FE2E_LOOP_BIN/$b"
      if [ -x "$f" ]; then cat "$f" > "$LOOP/$b"; fi
    done
  fi
  if [ -n "${FE2E_LOOP_TREE:-}" ] && [ -d "$FE2E_LOOP_TREE" ]; then
    git -C "$FE2E_LOOP_TREE" diff "$(git -C "$SRC" rev-parse HEAD)" -- . ':(exclude)crates' > "$LOOP/tree.patch"
    for s in "$A" "$B"; do
      local c="$WORK/$s/clone"
      git -C "$c" apply --3way "$LOOP/tree.patch" 2>>"$LOOP/apply.log" || git -C "$c" apply --reject "$LOOP/tree.patch" 2>>"$LOOP/apply.log" || true
      git -C "$c" add -A
      git -C "$c" diff --cached --quiet || git -C "$c" commit -q -m "loop: the fixer's tree, turn $LOOP_N"
      git -C "$c" push -q origin HEAD:main
      host_exec "$(drv_host "$s")" git -C /opt/goat pull -q --ff-only origin main
    done
  fi
}

# Wait for the fixer, install, resume; repeat until the resumed run no longer fails.
loop_wait_resume() { # $1 us, $2 them, $3 transcript
  local s="$1" them="$2" t="$3" side ph rc
  while :; do
    LOOP_N=$((LOOP_N + 1))
    oc_status "$s" "$them"
    loop_note "turn $LOOP_N: loop at phase $(nh_failed_phase "$s") on $s, last refusal: $(loop_refusal "$t") [transcript $t]"
    # Strict (CI): the first refusal is the result. Nothing is resumed.
    [ "$FE2E_NH_MODE" = loop ] || { NH_REFUSED=1; return 1; }
    rm -f "$LOOP/go"
    until [ -s "$LOOP/go" ]; do sleep 5; done
    read -r side ph < "$LOOP/go"; mv "$LOOP/go" "$LOOP/go.$LOOP_N"
    [ -n "$side" ] || side="$s"
    loop_install
    t="$RECORD/$OC_PREFIX-$side-loop$LOOP_N.log"
    rc=0
    # shellcheck disable=SC2046
    if [ -z "$ph" ] || [ "$ph" = 0 ]; then
      oc_run "$side" "$(other "$side")" "loop$LOOP_N" "$(other "$side")" $(oc_as "$side") || rc=$?
    else
      oc_run "$side" "$(other "$side")" "loop$LOOP_N" "$(other "$side")" --from "$ph" $(oc_as "$side") || rc=$?
    fi
    oc_status "$side" "$(other "$side")"
    loop_note "turn $LOOP_N: resumed $side --from ${ph:-0}: exit $rc; phases passed: $(oc_summary "$RECORD/status-$side.json")"
    s="$side"; them="$(other "$side")"
    [ "$rc" = 0 ] && return 0
    grep -q 'last run stopped' <(oc_summary "$RECORD/status-$side.json") && return 0
  done
}

nh_failed_phase() { # $1 us: the phase the last run failed at, from the status document
  python3 - "$RECORD/status-$1.json" <<'PY'
import json, sys
try:
    p = json.load(open(sys.argv[1]))["peerings"][0]
except Exception:
    print("?"); sys.exit(0)
failed = [x["phase"] for x in p.get("phases", []) if x.get("outcome") == "failed"]
print(failed[-1] if failed else "?")
PY
}

nh_phase14_outcome() { # $1 us: phase 14's outcome in the status document, or nothing
  python3 - "$RECORD/status-$1.json" <<'PY'
import json, sys
try:
    p = json.load(open(sys.argv[1]))["peerings"][0]
except Exception:
    sys.exit(0)
o = [x.get("outcome", "") for x in p.get("phases", []) if x.get("phase") == 14]
print(o[-1] if o else "")
PY
}

# Phase 14 in the command BLOCKED only on the counterparty's surfaces (the partner's REST,
# SPIRE, probes) is the designed production reading, lab-only here (the separate strict
# acceptance proves those): not a refusal. The journal records it as phase 14 "failed", or,
# from the release that tells the two apart, "waiting" (exit 3, GB-199); either is read the
# same, and the transcript must still show nothing but counterparty surfaces BLOCKED.
loop_only_counterparty() { # $1 us, $2 transcript
  case "$(nh_phase14_outcome "$1")" in
    failed) [ "$(nh_failed_phase "$1")" = 14 ] || return 1 ;;
    waiting) ;;
    *) return 1 ;;
  esac
  # Phase 14's own run only: after the last "=== loop: phase 14 ===" marker the wrapper wrote
  # (acceptance prints its rows before the command's banner and header), else the whole file.
  local t; t="$(sed 's/\x1b\[[0-9;]*m//g; s/\r//g' "$2" 2>/dev/null | awk '/^=== loop: phase 14 ===$/{buf=""; next} {buf=buf $0 "\n"} END{printf "%s", buf}')"
  # Here-strings, not pipes into grep -q: under pipefail the writer's SIGPIPE reads as no match.
  grep -qE '^ *FAIL ' <<<"$t" && return 1
  local other; other="$(grep -E '^ *(BLOCKED|SKIP) ' <<<"$t" | grep -vE 'counterparty|_THEIRS|their core|FED_PROBE' || true)"
  [ -z "$other" ]
}

NH_REFUSED=0
# Neither side moved: in strict and loop mode nothing is admitted another way by hand.
oc_admit_another_way() { # $1 us, $2 them, $3 round
  loop_note "phase 10 on $1 was not reached by the command; nothing is admitted by hand"
  return 1
}

# After each run: a failed run (not a stop for an operator step) waits for the fixer.
oc_after_run() { # $1 us, $2 them, $3 transcript
  oc_status "$1" "$2"
  if loop_only_counterparty "$1" "$3"; then
    loop_note "phase 14 in the command on $1: only the counterparty checks BLOCKED (lab-only, Q-14); not a refusal"
    return 1
  fi
  if grep -q 'last run failed' <(oc_summary "$RECORD/status-$1.json"); then
    loop_wait_resume "$1" "$2" "$3" || return 1
    return 0
  fi
  return 1
}
# Acceptance failed outside the command: the same wait, then acceptance runs once more.
oc_accept_retry() { # $1 us, $2 them, $3 acceptance output
  loop_wait_resume "$1" "$2" "$3" || return 1
  return 0
}
# A line the unit would write for the operator is, in the loop, a refusal the fixer sees too.
hand() {
  grep -qxF -- "$*" "$HAND" 2>/dev/null || printf '%s\n' "$*" >> "$HAND"
  case "$*" in *lab-only:*) ;; *) loop_note "hand-written (the unit supplied it to go on): $*" ;; esac
}

# Phase 14 inside the command, with the drill's reach into the partner's side (lab-only, Q-14:
# only a drill owns both sides). A plain run goes to phase 13 (--to 13); once both sides have
# passed 1-13, phase 14 runs on each as `goatbuilder federate <them> --from 14` with the partner's surfaces given and
# the partner's network reachable for that run only. Phases 1-13 never get the reach.
eval "$(declare -f oc_run | sed '1s/^oc_run /oc_run_base /')"
eval "$(declare -f oc_env_more | sed '1s/^oc_env_more /oc_env_more_base /')"
LOOP_REACH=""
oc_env_more() { # $1 us, $2 them
  oc_env_more_base "$@"
  [ -n "$LOOP_REACH" ] || return 0
  local s="$1" them="$2" far
  far="$(python3 -c 'import json,sys,ipaddress as i; ag=json.load(open(sys.argv[1])); h=list(i.ip_network(ag["transport"]["link_subnet"])); ps=[p["sovereign_id"] for p in ag["parties"]]; print(h[ps.index(sys.argv[2])])' "$WORK/$s/launcher-$them/agreement.json" "$s")"
  OC_ENV+=(-e FED_ACCEPTANCE_STRICT=1
    -e "FED_THEIRS_REST=http://127.0.0.1:8000" -e "FED_SSH_THEIRS=ssh://goat@$(core_ip "$them")"
    -e "FED_SPIRE_THEIRS=ssh goat@$(core_ip "$them") docker exec spire-server /opt/spire/bin/spire-server"
    -e FED_PROBE_LAUNCHER=/usr/local/sbin/fe2e-probe-launcher
    -e "FE2E_PL_SSH=goat@$(edge_ip "$them")" -e "FE2E_PL_REST_IP=$(edge_ip "$them")" -e "FE2E_PL_FAR_IP=$far"
    -e FE2E_PL_DIR=/root/fe2e-launcher -e FE2E_PL_PORT=8201 -e FE2E_PL_TEMPLATE=/root/fe2e-launcher/probe.json5 -e "FE2E_PL_IMAGE=$ZENOH_IMAGE")
}
loop_reach_on() { # $1 us, $2 them: the probes' identities and the reach, as oc_accept gives them
  local s="$1" them="$2" h pl pub; h="$(drv_host "$s")"; pl="$WORK/$s/launcher-$them"; mkdir -p "$pl"
  docker cp -q "$(drv_host "$them"):/var/lib/goat-federation/$s/pki/peering-outward.pem" "$pl/agreed-cert.pem"
  docker cp -q "$(drv_host "$them"):/var/lib/goat-federation/$s/pki/peering-outward-key.pem" "$pl/agreed-key.pem"
  cp "$WORK/$them/keys/forged.pem" "$pl/forged-cert.pem"; cp "$WORK/$them/keys/forged-key.pem" "$pl/forged-key.pem"
  cp "$WORK/$them/keys/impersonate-$s.pem" "$pl/impersonate-cert.pem"; cp "$WORK/$them/keys/impersonate-$s-key.pem" "$pl/impersonate-key.pem"
  cp "$WORK/$s/keys/ca.pem" "$pl/far-ca.pem"; chmod 0644 "$pl"/*
  docker cp -q "$(drv_host "$s"):/var/lib/goat-federation/$them/agreement.json" "$pl/agreement.json"
  host_exec "$h" install -d -m 0700 /root/fe2e-launcher
  for f in "$pl"/*-cert.pem "$pl"/*-key.pem "$pl/far-ca.pem"; do docker cp -q "$f" "$h:/root/fe2e-launcher/"; done
  docker cp -q "$HERE/lib/probe-launcher.sh" "$h:/usr/local/sbin/fe2e-probe-launcher"
  docker cp -q "$HERE/probe.json5" "$h:/root/fe2e-launcher/probe.json5"
  host_exec "$h" chmod 0755 /usr/local/sbin/fe2e-probe-launcher
  docker network connect "$NP-$them" "$h" >/dev/null 2>&1 || true
  pub="$(cat "$WORK/$s/keys/id_ed25519.pub")"
  for t in "$HP-$them-core" "$HP-$them-edge"; do docker exec "$t" sh -c "grep -qxF '$pub' /home/goat/.ssh/authorized_keys || printf '%s\n' '$pub' >> /home/goat/.ssh/authorized_keys"; done
  hand "values FED_THEIRS_REST, FED_SSH_THEIRS, FED_SPIRE_THEIRS, FED_PROBE_LAUNCHER and the drill's reach into the partner's network, given to phase 14 (\`goatbuilder federate $them --from 14\`) on $s: lab-only: only a drill owns both sides; a production operator holds only their own (docs/design/federation-acceptance.md)"
  LOOP_REACH=1
}
loop_reach_off() { # $1 us, $2 them
  local s="$1" them="$2" pub; pub="$(cat "$WORK/$s/keys/id_ed25519.pub")"
  for t in "$HP-$them-core" "$HP-$them-edge"; do
    docker exec "$t" sh -c "grep -vxF '$pub' /home/goat/.ssh/authorized_keys > /home/goat/.ssh/ak.tmp; cat /home/goat/.ssh/ak.tmp > /home/goat/.ssh/authorized_keys; rm -f /home/goat/.ssh/ak.tmp" || true
  done
  docker network disconnect "$NP-$them" "$(drv_host "$s")" >/dev/null 2>&1 || true
  LOOP_REACH=""
}
oc_run() { # $1 us, $2 them, $3 label, then `goatbuilder federate` arguments
  local s="$1" them="$2" label="$3" rc=0; shift 3
  # Strict: after the first refusal the command is not run again; the checks read the record.
  if [ "$NH_REFUSED" = 1 ]; then return 1; fi
  case " $* " in
    *" --from 14"*) loop_reach_on "$s" "$them"; oc_run_base "$s" "$them" "$label" "$@" || rc=$?; loop_reach_off "$s" "$them"; return "$rc" ;;
  esac
  [ "${1:-}" = "$them" ] || { oc_run_base "$s" "$them" "$label" "$@"; return; }
  if [ "$s" = "$B" ] && [ "${NH_ADMIT_TREE:-0}" = 1 ]; then
    # {B} admits through plan -> deploy: the plain run to phase 9, then `federate plan` and
    # `federate deploy` (which approves the plan and the admission at the terminal and runs
    # 10 to 13), then the plain run again for whatever is left before 14.
    oc_status "$s" "$them"
    if ! oc_passed "$RECORD/status-$s.json" 1 2 3 4 5 6 7 8 9; then
      oc_run_base "$s" "$them" "$label" "$@" --to 9 || rc=$?
      [ "$rc" = 0 ] || return "$rc"
      oc_status "$s" "$them"
      oc_passed "$RECORD/status-$s.json" 1 2 3 4 5 6 7 8 9 || return 0
    fi
    if ! oc_passed "$RECORD/status-$s.json" 10; then
      oc_run_base "$s" "$them" "$label-plan" plan "$them" || rc=$?
      [ "$rc" = 0 ] || { cat "$RECORD/$OC_PREFIX-$s-$label-plan.log" >> "$RECORD/$OC_PREFIX-$s-$label.log" 2>/dev/null || true; return "$rc"; }
      # shellcheck disable=SC2046
      oc_run_base "$s" "$them" "$label-deploy" deploy "$them" $(oc_as "$s") || rc=$?
      { echo "=== loop: deploy ==="; cat "$RECORD/$OC_PREFIX-$s-$label-deploy.log"; } >> "$RECORD/$OC_PREFIX-$s-$label.log" 2>/dev/null || true
      [ "$rc" = 0 ] || return "$rc"
      oc_status "$s" "$them"
      oc_passed "$RECORD/status-$s.json" 1 2 3 4 5 6 7 8 9 10 11 12 13 && return 0
      set -- "$them" --from 11
    fi
  fi
  oc_run_base "$s" "$them" "$label" "$@" --to 13 || rc=$?
  [ "$rc" = 0 ] || return "$rc"
  # Phase 14 waits for the both-sides step (oc_drive below): a side that ran ahead of the
  # other's admission would fail delivery by design.
  return "$rc"
}

# Once both sides have passed 1-13, phase 14 runs again on both, so that each side's acceptance
# is read after the other has admitted it (a side that ran ahead failed delivery by design).
eval "$(declare -f oc_drive | sed '1s/^oc_drive /oc_drive_base /')"
oc_drive() { # $1 first side, $2 second side
  oc_drive_base "$@"
  local s them rc t
  for s in "$1" "$2"; do
    them="$(other "$s")"
    oc_passed "$RECORD/status-$s.json" 1 2 3 4 5 6 7 8 9 10 11 12 13 || continue
    rc=0
    # shellcheck disable=SC2046
    oc_run "$s" "$them" "both-14" "$them" --from 14 $(oc_as "$s") || rc=$?
    t="$RECORD/$OC_PREFIX-$s-both-14.log"
    oc_status "$s" "$them"
    loop_note "phase 14 on $s after both sides passed 13: exit $rc; $(sed 's/\x1b\[[0-9;]*m//g' "$t" | grep -m1 'acceptance-counts' || echo 'no acceptance-counts line')"
    if [ "$rc" != 0 ] && ! loop_only_counterparty "$s" "$t"; then loop_wait_resume "$s" "$them" "$t"; fi
  done
}
