#!/usr/bin/env bash
# units/pair.sh — two sovereigns, the full fourteen phases through one avenue, then the
# checks that need only that one federated pair. Sourced by lib/unit.sh.
#
# Avenue goatbuilder: scenario 1, A listens and goes first (steps s1-*).
# Avenue script:      scenario 2, B listens and goes first (steps s2-*).
# Scenarios that read this unit (scenarios/catalog.yaml): two-party-full, direct-path,
# persistent-reception, republisher-boundary, out-of-lane-refused, identity-refused,
# acceptance-exit-status, rerun-every-phase.

pair_first() { [ "$AVENUE" = goatbuilder ] && echo "$A" || echo "$B"; }
pair_n() { [ "$AVENUE" = goatbuilder ] && echo 1 || echo 2; }

# Sample a key on one core, read it back from the other core's store. $1 sender, $2 set
# under release/, $3 receiver, $4 nonce. Exit 0 when the exact value is stored there.
put_release() { curl -s -m 8 -X PUT -d "$4" "http://$(core_ip "$1"):8000/release/$2/$4" -o /dev/null -w '%{http_code}'; }
stored_at() { # $1 receiver, $2 key: prints the stored value or nothing
  # The REST plugin returns each value base64-encoded (acceptance.sh decodes it the same way).
  curl -s -m 5 "http://$(core_ip "$1"):8000/$2" | python3 -c 'import base64,json,sys
try: d=json.load(sys.stdin)
except Exception: sys.exit(0)
for r in d if isinstance(d,list) else []:
    v=r.get("value","")
    try: v=base64.b64decode(v, validate=True).decode()
    except Exception: pass
    print(v)'
}
export -f put_release stored_at

unit_main() {
  local n first second; n="$(pair_n)"; first="$(pair_first)"; second="$(other "$first")"
  scenario "$n" "$first" "$AVENUE"
  pair_direct_path "$n" "$first" "$second"
  pair_acceptance_exit "$n" "$first" "$second"
  pair_out_of_lane "$n" "$first" "$second"
  pair_wrong_ca "$n" "$first" "$second"
  pair_rerun_every_phase "$n" "$first" "$second"
  # Persistent reception and the republisher boundary read the receiving core's own store,
  # so they run in units/pair-role.sh, where the ref's goat-zenoh role renders that store;
  # the hand-run core here takes its stores from core-seed.json5.
}

pair_direct_path() { # the listener's endpoint is the far edge's own boundary address: no relay between
  local n="$1" first="$2" second="$3"
  rec "s$n-direct-path-endpoint-is-the-listener" --expect-in "$(boundary_ip "$first"):$(wgport "$first" "$second")" -- \
    docker exec "$HP-$second-edge" wg show "$(iface "$second" "$first")" endpoints
  # The contrast with forwarder-path: the listener sees the dialer at the dialer's own
  # boundary address, and has no carrier interface.
  rec "s$n-direct-path-listener-sees-the-dialer" --expect-in "$(boundary_ip "$second"):" -- bash -c \
    "docker exec $HP-$first-edge wg show $(iface "$first" "$second") endpoints; ! docker exec $HP-$first-edge ip link show wg-carry0 >/dev/null 2>&1"
}

# Persistent reception: no responder, no probe, no `goat sub`, nothing subscribed on the
# receiving side but its own configured store. A fresh value must still be stored there.
pair_persistent_reception() {
  local n="$1" first="$2" second="$3" s them nonce
  for s in "$first" "$second"; do
    docker exec "$HP-$s-core" pkill -f fed-responder.py >/dev/null 2>&1 || true
    docker exec "$HP-$s-edge" sh -c 'docker ps -q --filter name=fed-probe- | xargs -r docker stop' >/dev/null 2>&1 || true
  done
  rec "s$n-no-diagnostic-subscriber-runs" -- bash -c \
    "for s in $first $second; do docker exec $HP-\$s-core pgrep -f fed-responder.py && exit 1; docker exec $HP-\$s-edge docker ps -q --filter name=fed-probe- | grep . && exit 1; done; echo none; exit 0"
  for s in "$first" "$second"; do
    them="$(other "$s")"; nonce="pr$(date +%s)$RANDOM"
    rec "s$n-$s-to-$them-stored-without-a-subscriber" -- bash -c \
      "put_release $s all $nonce >/dev/null; for _ in \$(seq 1 30); do v=\$(stored_at $them from/$s/all/$nonce); [ \"\$v\" = $nonce ] && { echo stored: \$v; exit 0; }; sleep 1; done; echo 'not stored at $them: from/$s/all/$nonce'; exit 1"
  done
}

# The republisher lifts only the sets the agreement offers: a set staged beside a valid
# control does not cross, while the control does.
pair_republisher_boundary() {
  local n="$1" first="$2" second="$3" s them c x
  for s in "$first" "$second"; do
    them="$(other "$s")"; c="rbc$(date +%s)$RANDOM"; x="rbx$(date +%s)$RANDOM"
    rec "s$n-$s-control-crosses-the-republisher" -- bash -c \
      "put_release $s all $c >/dev/null; put_release $s not-offered $x >/dev/null; for _ in \$(seq 1 30); do [ \"\$(stored_at $them from/$s/all/$c)\" = $c ] && { echo control stored; exit 0; }; sleep 1; done; exit 1"
    rec "s$n-$s-unoffered-set-does-not-cross" -- bash -c \
      "sleep 5; v=\$(stored_at $them 'from/$s/not-offered/**'); echo \"stored='\$v'\"; case \"\$v\" in *$x*) exit 1 ;; esac; docker exec $HP-$s-core docker logs goat-fed-republisher-$them 2>&1 | grep -c 'not-offered' | grep -qx 0"
  done
}

# Out-of-lane writes through the agreed identity's own session, beside an allowed control:
# the receiving edge's allow and deny records are the evidence, and the receiving core
# holds the control and neither refused key.
pair_out_of_lane() {
  local n="$1" first="$2" second="$3" s them port nonce since log zid
  for s in "$first" "$second"; do
    them="$(other "$s")"; port="$(probe_port "$s" "$them" 8101)"; nonce="ool$(date +%s)$RANDOM"
    zid="$(probe_zid "$s-$them" agreed)"
    docker exec "$HP-$s-edge" docker start "fed-probe-$them-agreed" >/dev/null 2>&1 || true
    sleep 4
    since="$(docker exec "$HP-$them-edge" date -u +%Y-%m-%dT%H:%M:%SZ)"
    for k in "from/$s/all/$nonce" "ops/fabric/$nonce" "from/$them/all/$nonce"; do
      curl -s -m 5 -X PUT -d "$nonce" "http://$(edge_ip "$s"):$port/$k" -o /dev/null || true
    done
    sleep 4
    log="$RECORD/s$n-$s-to-$them-out-of-lane-edge.log"
    docker exec "$HP-$them-edge" docker logs --since "$since" "goat-zenoh-edge-$s" > "$log" 2>&1 || true
    rec "s$n-$s-to-$them-edge-allowed-the-control" --expect-in "authorized" -- bash -c \
      "grep -E 'is authorized to Put' '$log' | head -3; grep -qE 'is authorized to Put' '$log'"
    rec "s$n-$s-to-$them-edge-denied-out-of-lane" -- bash -c \
      "grep -E 'unauthorized to Put|is not authorized|Deny' '$log' | head -5; [ \$(grep -cE 'unauthorized to Put|is not authorized|Deny' '$log') -ge 2 ]"
    rec "s$n-$s-to-$them-core-holds-control-not-refused" -- bash -c \
      "c=\$(stored_at $them from/$s/all/$nonce); o=\$(stored_at $them ops/fabric/$nonce); p=\$(stored_at $them from/$them/all/$nonce); echo \"control=\$c ops=\$o spoof=\$p\"; [ \"\$c\" = $nonce ] && case \"\$o\$p\" in *$nonce*) exit 1 ;; esac"
  done
}

# A certificate from a CA neither party trusts: the far edge refuses the session, and no
# write made through it is stored.
pair_wrong_ca() {
  local n="$1" first="$2" second="$3" s them d port nonce
  s="$first"; them="$second"; d="$WORK/wrongca"; mkdir -p "$d"
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -keyout "$d/ca-key.pem" -out "$d/ca.pem" -days 30 -subj "/O=nobody/CN=nobody Root CA" 2>/dev/null
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -keyout "$d/key.pem" -out "$d/req.csr" -subj "/CN=fed.$s.edge.01" 2>/dev/null
  printf 'subjectAltName=DNS:fed.%s.edge.01\nextendedKeyUsage=clientAuth\n' "$s" > "$d/ext"
  openssl x509 -req -in "$d/req.csr" -CA "$d/ca.pem" -CAkey "$d/ca-key.pem" -CAcreateserial -out "$d/cert.pem" -days 30 -extfile "$d/ext" 2>/dev/null
  chmod 0644 "$d"/*.pem
  # Not under /tmp: systemd mounts a tmpfs there inside the host, and docker cp writes beneath it.
  docker cp -q "$d/cert.pem" "$HP-$s-edge:/root/wrongca-cert.pem"; docker cp -q "$d/key.pem" "$HP-$s-edge:/root/wrongca-key.pem"
  if ! docker exec "$HP-$s-edge" test -s /root/wrongca-key.pem; then
    mark identity-refused.wrong-ca BLOCKED "the wrong-CA certificate could not be placed on $s's edge host"
    return 0
  fi
  # The evidence is the far edge's own TLS answer: a handshake from our edge to its fabric
  # port across the tunnel with the wrong-CA certificate must draw a fatal alert, and the same
  # handshake with the agreed certificate (the control) must not. The client stays open a few
  # seconds, because under TLS 1.3 the refusal of a client certificate arrives after the
  # handshake. Both transcripts are kept as evidence.
  local far w; w="/var/lib/goat-federation/$them"
  far="$(python3 -c 'import json,sys,ipaddress as i; ag=json.load(open(sys.argv[1])); h=list(i.ip_network(ag["transport"]["link_subnet"])); ps=[p["sovereign_id"] for p in ag["parties"]]; print(h[ps.index(sys.argv[2])])' "$WORK/$s/agreement-$them.json" "$them")"
  local hs="(sleep 4) | timeout 15 openssl s_client -connect $far:7448 -CAfile $w/pki/partner-ca.pem"
  rec "s$n-wrong-ca-session-is-refused" -- bash -c \
    "docker exec $HP-$s-edge bash -c '$hs -cert $w/pki/peering-outward.pem -key $w/pki/peering-outward-key.pem' > '$RECORD/s$n-tls-agreed.log' 2>&1;
     docker exec $HP-$s-edge bash -c '$hs -cert /root/wrongca-cert.pem -key /root/wrongca-key.pem' > '$RECORD/s$n-tls-wrong-ca.log' 2>&1;
     pat='alert (unknown ca|bad certificate|certificate unknown|certificate required|decrypt error|handshake failure)';
     echo \"agreed: \$(grep -iE \"\$pat\" '$RECORD/s$n-tls-agreed.log' | head -1)\"; echo \"wrong ca: \$(grep -iE \"\$pat\" '$RECORD/s$n-tls-wrong-ca.log' | head -1)\";
     grep -q 'CONNECTED' '$RECORD/s$n-tls-agreed.log' || { echo 'the control did not reach the far edge: no evidence'; exit 1; };
     ! grep -qiE \"\$pat\" '$RECORD/s$n-tls-agreed.log' && grep -qiE \"\$pat\" '$RECORD/s$n-tls-wrong-ca.log'"
}

# Acceptance with required checks unable to run (the probes and the edge log withheld), and
# without FED_ACCEPTANCE_STRICT, while everything else still passes (responders running, so
# fail=0): only the skips can make it exit non-zero, and it must.
pair_acceptance_exit() {
  local n="$1" first="$2" second="$3" s them w
  s="$first"; them="$second"; w="$WORK/$s/fedwork-exit"
  mkdir -p "$w"; cp "$WORK/$s/agreement-$them.json" "$w/agreement.json"
  rec "s$n-acceptance-with-a-skipped-check-does-not-exit-0" -- bash -c \
    "cd '$WORK/$s/clone' && env FED_WORK='$w' GOAT_AWS_MODE=none FED_OURS_REST=http://$(core_ip "$s"):8000 FED_THEIRS_REST=http://$(core_ip "$them"):8000 \
       '$WORK/$s/clone/ops/federation/standup/acceptance.sh' '$w/agreement.json' --as '$s' > '$RECORD/s$n-acceptance-nonstrict.out' 2>&1; rc=\$?; tail -5 '$RECORD/s$n-acceptance-nonstrict.out'; echo exit=\$rc;
       c=\$(grep -o 'acceptance-counts: pass=[0-9]* fail=[0-9]* skip=[0-9]*' '$RECORD/s$n-acceptance-nonstrict.out' | tail -1); echo \"\$c\";
       case \"\$c\" in *' fail=0 skip=0') echo 'no check was skipped: this run cannot show the rule'; exit 1 ;; *' fail=0 skip='*) ;; *) echo 'a check failed: the exit status would not come from the skip'; exit 1 ;; esac;
       [ \$rc -ne 0 ]"
}

# Every phase again, on a running peering: nothing duplicated, the working keys kept.
pair_rerun_every_phase() {
  local n="$1" first="$2" second="$3" s them
  for s in "$first" "$second"; do
    them="$(other "$s")"
    snap "$s" "$them" "" > "$RECORD/s$n-$s-before-rerun.txt"
    keys_of "$s" "$them" > "$RECORD/s$n-$s-keys-before-rerun.txt"
  done
  for s in "$first" "$second"; do
    them="$(other "$s")"
    rec "s$n-$s-rerun-phases-1-to-4" -- bash -c "drive '$s' '$them' '$AVENUE' 1 4"
    rec "s$n-$s-rerun-phases-6-to-9" -- bash -c "drive '$s' '$them' '$AVENUE' 6 9"
    rec "s$n-$s-rerun-phases-11-to-13" -- bash -c "drive '$s' '$them' '$AVENUE' 11 13"
  done
  for s in "$first" "$second"; do
    them="$(other "$s")"
    snap "$s" "$them" "" > "$RECORD/s$n-$s-after-rerun.txt"
    keys_of "$s" "$them" > "$RECORD/s$n-$s-keys-after-rerun.txt"
    rec "s$n-$s-rerun-kept-the-working-keys" -- diff "$RECORD/s$n-$s-keys-before-rerun.txt" "$RECORD/s$n-$s-keys-after-rerun.txt"
    rec "s$n-$s-rerun-duplicated-nothing" -- bash -c \
      "docker exec $HP-$s-edge sh -c 'docker ps -a --format {{.Names}} | sort | uniq -d; ip -o link | grep -c wg-fed' | tee /dev/stderr | tail -1 | grep -qx 1 && docker exec $HP-$s-core sh -c 'docker ps -a --format {{.Names}} | grep -c goat-fed-republisher-' | grep -qx 1"
  done
  rec "s$n-round-trip-after-rerun" --expect-in "PASS" -- bash -c "round_trip_eventually '$first' '$second'"
  # New keys only on explicit rotation: a check only where a driver offers rotation.
  if grep -rqs -- '--rotate' "$SRC/ops/scripts/peering-standup.sh" "$SRC/crates/goatbuilder/src"; then
    mark rerun-every-phase.rotation-explicit-only BLOCKED "a driver names --rotate; this harness does not drive rotation yet"
  else
    mark rerun-every-phase.rotation-explicit-only BLOCKED "neither driver offers key rotation (mint-peering-leaves.sh --rotate is never passed), so rotation on request cannot be shown"
  fi
}

keys_of() { # $1 us, $2 them: the fingerprints of every key the peering works with
  docker exec "$HP-$1-edge" sh -c "
    wg show $(iface "$1" "$2") public-key
    for f in /var/lib/goat-federation/$2/pki/*.pem; do case \"\$f\" in *key*) openssl pkey -in \"\$f\" -pubout 2>/dev/null | sha256sum | sed \"s|-|\$f|\" ;; esac; done"
}
