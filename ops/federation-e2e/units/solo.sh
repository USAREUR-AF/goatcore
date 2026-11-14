#!/usr/bin/env bash
# units/solo.sh — one lab sovereign whose federation partner is outside the lab
# (FE2E_PARTNER=external, lib/engine.sh). The lab runs only its own side of phases 1-14;
# whatever crosses to the partner other than the tunnel crosses as files through two
# folders, and each human step is a pause that waits for a file, never a prompt.
# Sourced by lib/unit.sh. Scenario: solo-external-partner (scenarios/catalog.yaml).
#
# The lab sovereign is A and the partner B (env run --parties "<lab> <partner>"). Inputs:
#   FE2E_PARTNER_FACTS   key=value file about the partner (# starts a comment):
#                          sovereign_id=<partner id>          must equal B
#                          edge_address=<address>             the partner edge's public address
#                          edge_port=<udp port>               its inner WireGuard port
#                          listens=true|false                 whether the partner edge listens
#                          release=<set> [<set>...]           the release sets it offers
#   FE2E_EXCHANGE_OUT    the folder this unit writes to (the laptop reads it)
#   FE2E_EXCHANGE_IN     the folder this unit reads (the laptop fills it)
#   FE2E_SOLO_PUBLIC_ADDR, FE2E_SOLO_PUBLISH, FE2E_SOLO_WAN, FE2E_SOLO_WAN_IP
#                        this edge's public side (lib/engine.sh, solo_endpoint). The lab
#                        edge dials when the partner listens, and listens when it does not.
#   FE2E_SOLO_STEP_TIMEOUT  seconds one pause waits (default 21600, 6 h); then the unit
#                        stops and every later check reads BLOCKED with the step named.
#
# OUT receives, in order: <A>.canonical.json, <A>-ca.pem, <A>.spiffe-bundle and
# <A>-voice-call.txt (phase 5); <A>-agreement-digest.txt (after compose);
# acceptance-<A>.json (phase 14); <A>-sample-N.txt, <A>-received-N.txt and
# <A>-core-restarted.txt (persistent reception); <step>.waiting while a pause waits.
# IN is expected to receive: <B>.canonical.json, <B>-ca.pem, <B>.spiffe-bundle; then
# voice-call.ok (signing_key=, ca_fingerprint= of the partner, as read on the call);
# digest.ok (the partner's agreement digest); admit.ok; tunnel.ok; accept.ok; persist.ok;
# optionally acceptance-<B>.json; and the partner's own <B>-sample-N.txt,
# <B>-received-N.txt and <B>-core-restarted.txt. Two solo units with crossed folders
# (one's OUT is the other's IN) play both sides lab to lab (tests/solo-pair.sh).

# The pair unit's helpers (put_release, stored_at) and the role core.
# shellcheck source=pair-role.sh
. "$HERE/units/pair-role.sh"

SOLO_N=0
OUT="${FE2E_EXCHANGE_OUT:-}"; IN="${FE2E_EXCHANGE_IN:-}"

kv_get() { # $1 file, $2 key: the last value, comment and surrounding blanks removed
  sed -n "s/^[[:space:]]*$2[[:space:]]*=[[:space:]]*//p" "$1" 2>/dev/null | sed 's/[[:space:]]*#.*$//; s/[[:space:]]*$//' | tail -1
}
fingerprint_of() { echo "sha256:$(openssl x509 -in "$1" -outform DER 2>/dev/null | sha256sum | cut -d' ' -f1)"; }
put_out() { # $1 source file, $2 name in OUT: whole or not at all, for a reader polling the folder
  cp "$1" "$OUT/.$2.tmp" && mv "$OUT/.$2.tmp" "$OUT/$2"
}
text_out() { # $1 name in OUT, text on stdin
  cat > "$OUT/.$1.tmp" && mv "$OUT/.$1.tmp" "$OUT/$1"
}
# The partner's package is the partner the facts describe: id, offer and release sets.
package_matches() { # $1 package, $2 id, $3 endpoint, $4 listens, $5 release sets
  python3 - "$@" <<'PY'
import json, sys
p = json.load(open(sys.argv[1])); want_id, ep, listens, rel = sys.argv[2:6]
t = p.get("transport_offer") or {}; d = p.get("data_offer") or {}
got = {"sovereign_id": p.get("sovereign_id"), "endpoint": t.get("endpoint"),
       "listener": str(t.get("listener")).lower(), "release": sorted(d.get("release") or [])}
want = {"sovereign_id": want_id, "endpoint": ep, "listener": listens, "release": sorted(rel.split())}
for k in want:
    print("%-12s package=%s facts=%s" % (k, got[k], want[k]))
sys.exit(0 if got == want else 1)
PY
}
export -f kv_get fingerprint_of package_matches

# A pause at a human step: OUT/<step>.waiting says what it waits for, until every named
# file is in IN, or the step times out and the unit stops (its later checks BLOCKED).
solo_wait() { # $1 step, $2 what it waits for, then the files under IN
  local step="$1" what="$2" f missing t0 limit="${FE2E_SOLO_STEP_TIMEOUT:-21600}"; shift 2
  t0="$(date +%s)"
  { echo "step: $step"; echo "waiting for: $what"; echo "files in the exchange folder IN:"
    for f in "$@"; do echo "  $f"; done
    echo "since: $(date -u +%Y-%m-%dT%H:%M:%SZ)"; echo "timeout: $limit s"; } | text_out "$step.waiting"
  say "solo: step $step waits for $what ($*)"
  while :; do
    missing=""
    for f in "$@"; do
      # An .ok file may be empty; every other file has content.
      case "$f" in *.ok) [ -e "$IN/$f" ] || missing="$missing $f" ;; *) [ -s "$IN/$f" ] || missing="$missing $f" ;; esac
    done
    [ -z "$missing" ] && break
    if [ $(( $(date +%s) - t0 )) -ge "$limit" ]; then
      rm -f "$OUT/$step.waiting"
      die "step $step timed out after $limit s waiting for$missing in the exchange folder ($what)"
    fi
    sleep 5
  done
  sleep 2 # a file still being copied in
  rm -f "$OUT/$step.waiting"
  echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) $step waited $(( $(date +%s) - t0 )) s for $*" >> "$RECORD/solo-pauses.log"
}

solo_facts() { # read and check the partner's facts and this edge's public side
  local f="${FE2E_PARTNER_FACTS:-}"
  [ -n "$f" ] && [ -s "$f" ] || die "FE2E_PARTNER_FACTS names no readable facts file about the partner"
  [ -n "$OUT" ] && [ -n "$IN" ] || die "set FE2E_EXCHANGE_OUT and FE2E_EXCHANGE_IN, the two exchange folders"
  mkdir -p "$OUT" "$IN" || die "the exchange folders $OUT and $IN cannot be created"
  SOLO_THEM_ID="$(kv_get "$f" sovereign_id)"; SOLO_THEM_ADDR="$(kv_get "$f" edge_address)"
  SOLO_THEM_PORT="$(kv_get "$f" edge_port)"; SOLO_THEM_LISTENS="$(kv_get "$f" listens)"
  SOLO_THEM_RELEASE="$(kv_get "$f" release)"
  cp "$f" "$RECORD/partner-facts.txt"
  [ "$SOLO_THEM_ID" = "$B" ] || die "the facts name partner '$SOLO_THEM_ID', but this run's partner (B, env --parties) is '$B'"
  [ -n "$SOLO_THEM_ADDR" ] || die "the facts give no edge_address"
  case "$SOLO_THEM_PORT" in ''|*[!0-9]*) die "the facts give no numeric edge_port" ;; esac
  case "$SOLO_THEM_LISTENS" in true) SOLO_WE_LISTEN=false ;; false) SOLO_WE_LISTEN=true ;; *) die "the facts say listens=true or listens=false, not '$SOLO_THEM_LISTENS'" ;; esac
  [ -n "$SOLO_THEM_RELEASE" ] || die "the facts name no release set the partner offers"
  if [ "$SOLO_WE_LISTEN" = true ] && [ -z "${FE2E_SOLO_PUBLISH:-}" ] && [ -z "${FE2E_SOLO_WAN:-}" ]; then
    die "the partner does not listen, so this edge must: set FE2E_SOLO_PUBLISH (a UDP port this machine publishes) or FE2E_SOLO_WAN"
  fi
  [ -n "$(solo_public_addr)" ] || die "set FE2E_SOLO_PUBLIC_ADDR, the address the partner's edge sees this edge at"
  # Files a pause would take for an answer must not be left from an earlier run.
  local stale
  stale="$(cd "$IN" && ls -1 ./*.ok "$B"-sample-*.txt "$B"-received-*.txt "$B"-core-restarted.txt "acceptance-$B.json" 2>/dev/null | tr '\n' ' ' || true)"
  [ -z "$stale" ] || die "the exchange folder IN ($IN) holds answers left from an earlier run: $stale; start with an empty folder"
  rm -f "$OUT"/*.waiting
  export SOLO_THEM_ID SOLO_THEM_ADDR SOLO_THEM_PORT SOLO_THEM_LISTENS SOLO_THEM_RELEASE SOLO_WE_LISTEN
  rec "solo-partner-facts-are-consistent" -- bash -c "echo 'partner $SOLO_THEM_ID at $SOLO_THEM_ADDR:$SOLO_THEM_PORT, listens=$SOLO_THEM_LISTENS, offers $SOLO_THEM_RELEASE'; echo 'this edge ($A) listens=$SOLO_WE_LISTEN, offers $(solo_endpoint "$A" "$B")'"
}

# --- phase 5 by hand, and the voice call ------------------------------------------------
solo_exchange() {
  local s="$A" them="$B" h w; h="$(drv_host "$A")"; w="/var/lib/goat-federation/$B"
  mkdir -p "$WORK/$s/out"
  docker cp -q "$h:$w/ours.canonical.json" "$WORK/$s/out/package.json" || die "phase 5 left no package at $w/ours.canonical.json"
  docker cp -q "$h:$w/ours.spiffe-bundle" "$WORK/$s/out/bundle" || die "phase 5 left no SPIFFE bundle at $w/ours.spiffe-bundle"
  put_out "$WORK/$s/out/package.json" "$s.canonical.json"
  put_out "$WORK/$s/keys/ca.pem" "$s-ca.pem"
  put_out "$WORK/$s/out/bundle" "$s.spiffe-bundle"
  { echo "# Read aloud on the voice call; the partner reads theirs back."
    echo "sovereign_id=$s"
    echo "signing_key=$(cat "$WORK/$s/keys/signing.pub")"
    echo "ca_fingerprint=$(fingerprint_of "$WORK/$s/keys/ca.pem")"; } | text_out "$s-voice-call.txt"
  rec "solo-our-files-are-in-out" -- bash -c "cd '$OUT' && ls -l '$s.canonical.json' '$s-ca.pem' '$s.spiffe-bundle' '$s-voice-call.txt' && cat '$s-voice-call.txt'"

  solo_wait exchange "the partner's package, CA certificate and SPIFFE bundle" "$them.canonical.json" "$them-ca.pem" "$them.spiffe-bundle"
  cp "$IN/$them.canonical.json" "$IN/$them-ca.pem" "$IN/$them.spiffe-bundle" "$RECORD/" 2>/dev/null || true
  docker cp -q "$IN/$them.canonical.json" "$h:$w/theirs.canonical.json"
  docker cp -q "$IN/$them-ca.pem" "$h:$w/theirs-ca.pem"
  docker cp -q "$IN/$them.spiffe-bundle" "$h:$w/theirs.spiffe-bundle"
  rec "solo-partner-files-are-in-place" -- docker exec "$h" sh -c \
    "test -s $w/theirs.canonical.json && test -s $w/theirs-ca.pem && test -s $w/theirs.spiffe-bundle && ls -l $w/theirs*"
  # The package is the partner the facts describe: its id, its offer, its release sets.
  rec "solo-partner-package-matches-the-facts" -- bash -c \
    "package_matches '$IN/$them.canonical.json' '$them' '$SOLO_THEM_ADDR:$SOLO_THEM_PORT' '$SOLO_THEM_LISTENS' '$SOLO_THEM_RELEASE'"

  # The voice call: the partner's key and CA fingerprint as read aloud, in voice-call.ok.
  solo_wait voice-call "the voice call: the partner's signing_key and ca_fingerprint as read aloud (our values are in OUT/$s-voice-call.txt)" voice-call.ok
  local key fp
  key="$(kv_get "$IN/voice-call.ok" signing_key)"; fp="$(kv_get "$IN/voice-call.ok" ca_fingerprint)"
  cp "$IN/voice-call.ok" "$RECORD/voice-call.ok"
  rec "solo-voice-call-matches-the-partner-files" -- bash -c \
    "got=\$(fingerprint_of '$IN/$them-ca.pem'); echo \"read aloud: key=$key ca=$fp\"; echo \"their CA file: \$got\";
     echo '$key' | grep -qE '^[A-Za-z0-9+/]{43}=\$' || { echo 'the signing key read aloud is not 44 base64 characters'; exit 1; }
     [ \"\$got\" = '$fp' ]"
  printf '%s=%s\n%s=%s\n' "$s" "$(cat "$WORK/$s/keys/signing.pub")" "$them" "$key" > "$WORK/$s/keys.txt"
  docker cp -q "$WORK/$s/keys.txt" "$h:$w/keys.txt"
}

# --- the agreement digest ---------------------------------------------------------------
solo_digest() {
  local h w ours theirs; h="$(drv_host "$A")"; w="/var/lib/goat-federation/$B"
  ours="sha256:$(docker exec "$h" sha256sum "$w/agreement.json" | cut -d' ' -f1)"
  echo "$ours" | text_out "$A-agreement-digest.txt"
  say "solo: our agreement digest is $ours"
  solo_wait digest "the partner's agreement digest (ours: $ours)" digest.ok
  theirs="$(grep -oE '[0-9a-f]{64}' "$IN/digest.ok" | head -1 || true)"
  rec "solo-agreement-digest-equals-the-partners" -- bash -c "echo 'ours:    $ours'; echo 'partner: sha256:$theirs'; [ 'sha256:$theirs' = '$ours' ]"
  [ "sha256:$theirs" = "$ours" ] || die "the agreement digests differ: ours $ours, the partner's sha256:${theirs:-none}; the parties did not compose one agreement"
}

# --- phase 13, run again while the partner's half is still coming up ---------------------
solo_phase13() { # $1 us, $2 them, $3 avenue: up to ten tries, 30 s apart; the last output printed
  local i out
  for i in 1 2 3 4 5 6 7 8 9 10; do
    if out="$(drive "$1" "$2" "$3" 13 13 2>&1)"; then printf '%s\n' "$out"; echo "phase 13 passed on try $i"; return 0; fi
    sleep 30
  done
  printf '%s\n' "$out"; echo "phase 13 did not pass in ten tries"; return 1
}

# --- phase 14: strict acceptance with this side's own surfaces only ----------------------
accept_solo() { # $1 us, $2 them, $3 avenue
  local s="$1" them="$2" avenue="$3" w; w="$WORK/$s/fedwork-$them"
  mkdir -p "$w"; cp "$WORK/$s/agreement-$them.json" "$w/agreement.json"
  local env=(
    "FED_WORK=$w" "GOAT_AWS_MODE=none" "FED_ACCEPTANCE_STRICT=1"
    "FED_OURS_REST=http://$(core_ip "$s"):8000"
    "FED_EDGE_LOG_OURS=docker exec $HP-$s-edge docker logs goat-zenoh-edge-$them"
    "FED_SPIRE_OURS=docker exec $HP-$s-core docker exec spire-server /opt/spire/bin/spire-server"
  )
  case "$avenue" in
    goatbuilder) env "${env[@]}" "$GB" -C "$WORK/$s/clone" --yes federate "$them" --as "$s" --work "$w" --from 14 --to 14 ;;
    script)      env "${env[@]}" FED_WORK="$w" "$WORK/$s/clone/ops/scripts/peering-standup.sh" "$them" --as "$s" --from 14 --to 14 ;;
  esac
}
# An acceptance result (goat-federation-acceptance/v1) judged for this agreement:
# exit 0 every required check passed; 1 a check failed, or the result is for another
# agreement; 3 nothing failed and every check that did not run lacked a surface only the
# partner holds; 4 nothing failed but some other check did not run.
accept_judge() { # $1 result.json, $2 our agreement digest
  python3 - "$1" "$2" <<'PY'
import json, re, sys
try:
    r = json.load(open(sys.argv[1]))
except (OSError, ValueError) as e:
    print("no readable acceptance result: %s" % e); sys.exit(4)
c = r.get("counts", {}); dig = (r.get("agreement") or {}).get("sha256") or ""
print("verdict %s (exit %s): %s" % (r.get("verdict"), r.get("exit"), r.get("why")))
print("counts: " + ", ".join("%s %s" % kv for kv in c.items()))
print("agreement %s, ours %s" % (dig, sys.argv[2]))
if dig.split(":")[-1] != sys.argv[2].split(":")[-1]:
    print("the result is for another agreement"); sys.exit(1)
if c.get("FAIL", 0):
    sys.exit(1)
if r.get("verdict") == "PASS" and not c.get("BLOCKED", 0) and not c.get("SKIP", 0):
    sys.exit(0)
partner = re.compile(r"FED_THEIRS_REST|FED_SPIRE_THEIRS|FED_PROBE_|counterparty surface|their core's REST")
other = []
for ck in r.get("checks", []):
    if ck.get("verdict") in ("BLOCKED", "SKIP"):
        why = (ck.get("reason") or "") + " " + (ck.get("title") or "")
        print("  %s %s: %s" % (ck.get("verdict"), ck.get("name"), why.strip()[:160]))
        if not partner.search(why):
            other.append(ck.get("name"))
if other:
    print("not run for a reason other than a partner-held surface: " + ", ".join(other)); sys.exit(4)
sys.exit(3)
PY
}
export -f solo_phase13 accept_solo accept_judge

solo_acceptance() {
  local s="$A" them="$B" ours rc res="$WORK/$A/fedwork-$B/evidence/acceptance/result.json" why
  ours="$(cat "$OUT/$A-agreement-digest.txt")"
  # A responder on our core acknowledges the partner's outbound round trip.
  host_exec "$HP-$s-core" sh -c 'cat > /usr/local/bin/fed-responder.py' < "$SRC/ops/federation/roundtrip/responder.py"
  docker exec -d "$HP-$s-core" python3 /usr/local/bin/fed-responder.py "from/$them" "release/all"
  rec "solo-acceptance-local-ran" --expect-in "acceptance-counts:" -- bash -c \
    "accept_solo '$s' '$them' '$AVENUE' > '$RECORD/solo-acceptance.out' 2>&1; rc=\$?; tail -40 '$RECORD/solo-acceptance.out'; echo exit=\$rc;
     grep -q 'acceptance-counts: pass=[0-9]* fail=0 ' '$RECORD/solo-acceptance.out'"
  [ -s "$res" ] && put_out "$res" "acceptance-$s.json" && cp "$res" "$RECORD/acceptance-$s.json"
  rc=0; accept_judge "$res" "$ours" > "$RECORD/solo-acceptance-judged.txt" 2>&1 || rc=$?
  case "$rc" in
    0|1|4) rec "solo-acceptance-strict-local" -- bash -c "cat '$RECORD/solo-acceptance-judged.txt'; [ $rc = 0 ]" ;;
    3)
      # Record the counts as a step, and the check as BLOCKED: one side alone cannot
      # supply the partner's surfaces, and the ref has no exchange of acceptance halves.
      rec "solo-acceptance-strict-local-counts" -- cat "$RECORD/solo-acceptance-judged.txt"
      why="$(grep -o 'acceptance-counts: .*' "$RECORD/solo-acceptance.out" | tail -1)"
      mark solo-external-partner.acceptance-strict-local BLOCKED \
        "strict acceptance from one side alone did not complete ($why): every check that did not run needs a surface only the partner holds (their core's REST, their probes, their SPIRE server), and the ref has no exchange of the partner's acceptance half (docs/design/federation-acceptance.md: the bilateral exchange is not built)"
      ;;
  esac
}

solo_partner_acceptance() { # the partner's result, if dropped in IN: peer-reported only
  local them="$B" ours rc f; f="$IN/acceptance-$B.json"; ours="$(cat "$OUT/$A-agreement-digest.txt")"
  if [ ! -s "$f" ]; then
    mark solo-external-partner.partner-acceptance SKIP "the partner dropped no acceptance-$them.json in the exchange folder"
    return 0
  fi
  cp "$f" "$RECORD/acceptance-$them.peer-reported.json"
  rc=0; accept_judge "$f" "$ours" > "$RECORD/solo-partner-acceptance-judged.txt" 2>&1 || rc=$?
  case "$rc" in
    3) rec "solo-partner-acceptance-counts" -- cat "$RECORD/solo-partner-acceptance-judged.txt"
       mark solo-external-partner.partner-acceptance BLOCKED \
         "peer-reported: the partner's strict acceptance did not complete, and every check that did not run needs a surface only this side holds (the bilateral exchange is not built)" ;;
    *) rec "solo-partner-acceptance" -- bash -c "echo 'peer-reported by $them, not observed here'; cat '$RECORD/solo-partner-acceptance-judged.txt'; [ $rc = 0 ]" ;;
  esac
}

# --- persistent reception, both ways, by file -------------------------------------------
# Each side stages a sample on its own core with nothing subscribed on either side but
# the stores, and names it in OUT; the partner reads it from its own store and reports
# what it read. Our store's reading of the partner's sample is observed here; the
# partner's reading of ours is peer-reported. Then each side restarts its receiving core
# (not its edge) and the exchange runs again.
solo_sample() { # $1 round: stage a sample and name it in OUT
  local s="$A" nonce key; nonce="solo$1$(date +%s)$RANDOM"; key="from/$s/all/$nonce"
  put_release "$s" all "$B" "$nonce" > /dev/null
  printf 'key=%s\nvalue=%s\nstaged_as=release/all/%s\nstaged_at=%s\n' "$key" "$nonce" "$nonce" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" | text_out "$s-sample-$1.txt"
  # Staged again at 15 s and 30 s, the same value: a sending core that restarted may not
  # have its republisher back yet. One stored value is all the partner reads.
  ( sleep 15; put_release "$s" all "$B" "$nonce"; sleep 15; put_release "$s" all "$B" "$nonce" ) > /dev/null 2>&1 &
}
solo_receive() { # $1 round, $2 step name: read the partner's sample from our store, report it
  local s="$A" them="$B" key value got=""
  solo_wait "sample-$1" "the partner's sample $1, staged on its core" "$them-sample-$1.txt"
  key="$(kv_get "$IN/$them-sample-$1.txt" key)"; value="$(kv_get "$IN/$them-sample-$1.txt" value)"
  case "$key" in "from/$them/"*) ;; *) key="from/$them/(not-a-wire-key:$key)" ;; esac
  rec "$2" -- bash -c \
    "for _ in \$(seq 1 60); do v=\$(stored_at $s '$key'); [ -n '$value' ] && [ \"\$v\" = '$value' ] && { echo \"stored at $s: $key = \$v\"; exit 0; }; sleep 1; done; echo 'not stored at $s: $key'; exit 1"
  got="$(stored_at "$s" "$key" | head -1 || true)"
  printf 'key=%s\nexpected=%s\nread=%s\nresult=%s\nobserved_at=%s\n' "$key" "$value" "$got" \
    "$([ -n "$value" ] && [ "$got" = "$value" ] && echo stored || echo not-stored)" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" | text_out "$s-received-$1.txt"
}
solo_reported() { # $1 round, $2 step name: what the partner read of our sample
  local s="$A" them="$B" f
  solo_wait "received-$1" "the partner's report of our sample $1" "$them-received-$1.txt"
  f="$IN/$them-received-$1.txt"; cp "$f" "$RECORD/"
  rec "$2" -- bash -c \
    "echo 'peer-reported by $them, not observed here:'; cat '$f'; want=\$(kv_get '$OUT/$s-sample-$1.txt' value); key=\$(kv_get '$OUT/$s-sample-$1.txt' key);
     [ \"\$(kv_get '$f' key)\" = \"\$key\" ] && [ \"\$(kv_get '$f' read)\" = \"\$want\" ] && [ \"\$(kv_get '$f' result)\" = stored ]"
}

solo_persistence() {
  local s="$A" them="$B" before after
  docker exec "$HP-$s-core" pkill -f fed-responder.py >/dev/null 2>&1 || true
  rec "solo-no-diagnostic-subscriber-runs" -- bash -c \
    "docker exec $HP-$s-core pgrep -f fed-responder.py && exit 1; docker exec $HP-$s-edge docker ps -q --filter name=fed-probe- | grep . && exit 1; echo none; exit 0"
  solo_sample 1
  solo_receive 1 "solo-received-from-partner-without-a-subscriber"
  solo_reported 1 "solo-partner-received-without-a-subscriber"
  # Our receiving core restarts; nothing restarts our edge.
  before="$(docker exec "$HP-$s-edge" docker inspect -f '{{.State.StartedAt}}' "goat-zenoh-edge-$them" 2>/dev/null || true)"
  docker exec "$HP-$s-core" docker restart "$s-zenoh-router" >/dev/null 2>&1 || true
  for _ in $(seq 1 60); do curl -s -m 2 -o /dev/null "http://$(core_ip "$s"):8000/@/**" && break; sleep 1; done
  printf 'restarted=%s-zenoh-router\nrestarted_at=%s\n' "$s" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" | text_out "$s-core-restarted.txt"
  solo_wait partner-core-restarted "the partner's restart of its receiving core" "$them-core-restarted.txt"
  solo_sample 2
  solo_receive 2 "solo-received-from-partner-after-our-core-restarts"
  after="$(docker exec "$HP-$s-edge" docker inspect -f '{{.State.StartedAt}}' "goat-zenoh-edge-$them" 2>/dev/null || true)"
  rec "solo-our-edge-was-not-restarted" --expect-in "$before" -- echo "edge router goat-zenoh-edge-$them started at $after (before the core restart: $before)"
  solo_reported 2 "solo-partner-received-after-its-core-restarts"
}

unit_main() {
  FE2E_CORE=role; FE2E_PARTNER=external; export FE2E_CORE FE2E_PARTNER
  local s="$A" them="$B" h w i
  # shellcheck disable=SC2034 # NODIR is read by lib/engine.sh (nodir)
  set_partners "$A" "$B"; set_partners "$B" ""; export NODIR="$A:$B"
  export_helpers
  solo_facts
  say "solo: $s federates with $them, outside the lab; exchange folders OUT=$OUT IN=$IN"
  setup_hosts "$SOLO_N"
  h="$(drv_host "$s")"; w="/var/lib/goat-federation/$them"; i="$(iface "$s" "$them")"
  rec "solo-core-is-the-roles" \
    --expect-in "/$s-zenoh-router|zenoh-core|$s-zenoh|/opt/$s-zenoh/docker-compose.yml|/etc/$s-zenoh/config.json5|$ZENOH_IMAGE|true" \
    -- docker exec "$HP-$s-core" docker inspect -f '{{.Name}}|{{index .Config.Labels "org.goat.component"}}|{{index .Config.Labels "com.docker.compose.project"}}|{{index .Config.Labels "com.docker.compose.project.config_files"}}|{{range .Mounts}}{{if eq .Destination "/etc/zenoh/config.json5"}}{{.Source}}{{end}}{{end}}|{{.Config.Image}}|{{.State.Running}}' "$s-zenoh-router"

  prepare_edge "$s" "$them" "$SOLO_WE_LISTEN"
  rec "solo-phases-1-to-5" --expect-in "exchange packages by hand" -- bash -c "drive '$s' '$them' '$AVENUE' 1 5"
  docker exec "$h" test -s "$w/ours.canonical.json" || die "phases 1-5 wrote no package; nothing to exchange"
  solo_exchange

  rec "solo-phases-6-to-10" --expect-in "review what apply rendered" -- bash -c "drive '$s' '$them' '$AVENUE' 6 10"
  docker exec "$h" test -s "$w/agreement.json" || die "phases 6-10 composed no agreement"
  rec "solo-the-agreement-selected-wireguard" --expect-in '"kind":"wireguard"' -- docker exec "$h" cat "$w/agreement.json"
  solo_digest

  solo_wait admit "the operator's go-ahead to admit the partner's edge on our core (phase 10)" admit.ok
  review_and_admit "$s" "$them" "$SOLO_N"

  solo_wait tunnel "the partner ready for the tunnel (its phase 10 done)" tunnel.ok
  rec "solo-phases-11-to-12" --expect-in "open the wireguard transport" --expect-in "goat-fed-republisher-$them is running" -- bash -c "drive '$s' '$them' '$AVENUE' 11 12"
  rec "solo-republisher-maps-both-sets" --expect-in "release release/all/** -> wire from/$s/all/**" --expect-in "release release/$them/** -> wire from/$s/$them/**" \
    -- docker exec "$HP-$s-core" docker logs "goat-fed-republisher-$them"
  # The handshake waits as long as a pause: the partner's phase 11 may come later.
  rec "solo-wireguard-handshake" -- bash -c \
    "end=\$((\$(date +%s) + ${FE2E_SOLO_STEP_TIMEOUT:-21600})); while [ \$(date +%s) -lt \$end ]; do t=\$(docker exec $HP-$s-edge wg show $i latest-handshakes | cut -f2); [ \"\${t:-0}\" != 0 ] && break; sleep 5; done
     docker exec $HP-$s-edge wg show $i endpoints; docker exec $HP-$s-edge wg show $i latest-handshakes; [ \"\${t:-0}\" != 0 ]"
  rec "solo-phase-13" --expect-in "federate the SPIRE trust domains" -- bash -c "solo_phase13 '$s' '$them' '$AVENUE'"

  solo_wait accept "the partner ready for acceptance (its phase 13 done)" accept.ok
  docker cp -q "$h:$w/agreement.json" "$WORK/$s/agreement-$them.json"
  solo_acceptance

  solo_wait persist "the go-ahead for persistent reception (both acceptance runs finished)" persist.ok
  solo_partner_acceptance
  solo_persistence
  rec "solo-tunnel-carried-both-ways" -- bash -c \
    "docker exec $HP-$s-edge wg show $i transfer | awk '{print \"received\", \$2, \"sent\", \$3; if (\$2 < 1 || \$3 < 1) exit 1}'"
  collect "$SOLO_N"
}
