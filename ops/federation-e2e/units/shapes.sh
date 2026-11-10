#!/usr/bin/env bash
# units/shapes.sh — agreement shapes: one peering whose two sides declare different [data]
# halves, brought up through all fourteen phases, then checked against the shape the
# agreement declares. Sourced by units/shapes-oneway.sh and units/shapes-asym.sh, each of
# which sets the shape and calls shapes_main.
#
# A shape is three values per side, set before shapes_main:
#   SH_REL_<id>     the side's release sets, space separated staging prefixes without the
#                   /** tail (release/all), or empty for no send half
#   SH_ACC_<id>     yes | no: whether the side declares the receive half
# The engine's prepare_edge reads them as FE2E_RELEASE_<id> and FE2E_ACCEPT_<id>.
#
# Steps are s<n>-..., n being the unit's tag (h1 one-way, h2 asymmetric). A direction the
# agreement does not carry gets no crossing step: its checks are marked NOT APPLICABLE
# with the agreement digest as the reason. The sets a side does not offer are staged too,
# and must not cross.

put_key() { curl -s -m 8 -X PUT -d "$3" "http://$(core_ip "$1"):8000/$2" -o /dev/null -w '%{http_code}'; } # $1 core, $2 key, $3 value
stored_at() { # $1 receiver, $2 key or key expression: prints each stored value, decoded, or nothing
  # The REST plugin returns each value base64-encoded (acceptance.sh decodes it the same way).
  curl -s -m 5 "http://$(core_ip "$1"):8000/$2" | python3 -c 'import base64,json,sys
try: d=json.load(sys.stdin)
except Exception: sys.exit(0)
for r in d if isinstance(d,list) else []:
    v=r.get("value","")
    try: v=base64.b64decode(v, validate=True).decode()
    except Exception: pass
    print(r.get("key",""), v)'
}
sh_cross() { # $1 sender, $2 receiver, $3 set, $4 seconds: the set staged on $1 is stored at $2
  local nonce="sh$(date +%s)$RANDOM" t0 i=0
  t0="$(date +%s)"
  while [ $(($(date +%s) - t0)) -lt "$4" ]; do
    [ $((i % 15)) = 0 ] && put_key "$1" "release/$3/${SH_SUB-shapes/}$nonce" "$nonce" >/dev/null
    if [ "$(stored_at "$2" "from/$1/$3/${SH_SUB-shapes/}$nonce")" = "from/$1/$3/${SH_SUB-shapes/}$nonce $nonce" ]; then
      echo "stored at $2: from/$1/$3/${SH_SUB-shapes/}$nonce after $(($(date +%s) - t0))s"; return 0
    fi
    i=$((i + 1)); sleep 1
  done
  echo "not stored at $2 within $4s: from/$1/$3/${SH_SUB-shapes/}$nonce"; return 1
}
sh_stays() { # $1 sender, $2 receiver, $3 set not offered, [$4 an offered control set]
  local x="shx$(date +%s)$RANDOM" c="shc$(date +%s)$RANDOM" v
  put_key "$1" "release/$3/${SH_SUB-shapes/}$x" "$x" >/dev/null
  if [ -n "${4:-}" ]; then
    put_key "$1" "release/$4/${SH_SUB-shapes/}$c" "$c" >/dev/null
    for _ in $(seq 1 60); do [ "$(stored_at "$2" "from/$1/$4/${SH_SUB-shapes/}$c")" = "from/$1/$4/${SH_SUB-shapes/}$c $c" ] && break; sleep 1; done
    [ "$(stored_at "$2" "from/$1/$4/${SH_SUB-shapes/}$c")" = "from/$1/$4/${SH_SUB-shapes/}$c $c" ] || { echo "the control from/$1/$4/${SH_SUB-shapes/}$c did not arrive at $2: the path is dead, nothing to compare"; return 1; }
    echo "control stored at $2: from/$1/$4/${SH_SUB-shapes/}$c"
  else
    sleep 10; echo "no control: the agreement carries no $1->$2 flow"
  fi
  sleep 5
  v="$(stored_at "$2" "from/$1/**")"
  case "$v" in *"$x"*) echo "crossed: $(printf '%s\n' "$v" | grep "$x")"; return 1 ;; esac
  echo "from/$1/$3/${SH_SUB-shapes/}$x absent at $2 ($(printf '%s' "$v" | grep -c .) values stored under from/$1/**)"
}
export -f put_key stored_at sh_cross sh_stays
# The edge role's own refusals, named in a phase 11-12 failure so its reason says which.
EDGE_REFUSALS=(--expect-not-in "goat-zenoh-edge is missing required peering parameters"
  --expect-not-in "has NO connection to any core endpoint")

sh_rel() { eval "echo \"\${SH_REL_$1-}\""; }
sh_acc() { eval "echo \"\${SH_ACC_$1-yes}\""; }
# The direction s -> them exists only if s releases something and them accepts.
sh_flows() { [ -n "$(sh_rel "$1")" ] && [ "$(sh_acc "$2")" = yes ]; }
# Every set either side offers, plus one no one offers: the candidates a side must not send.
sh_all_sets() { { sh_rel "$A"; sh_rel "$B"; echo release/not-offered; } | tr ' ' '\n' | grep . | sort -u; }
sh_offers() { case " $(sh_rel "$1") " in *" $2 "*) return 0 ;; *) return 1 ;; esac; }

# The TOML for the engine's seam, from the shape.
sh_export_shape() {
  local s r arr
  for s in "$A" "$B"; do
    r="$(sh_rel "$s")"
    if [ -z "$r" ]; then arr=none; else
      arr="[$(for x in $r; do printf '"%s/**", ' "$x"; done | sed 's/, $//')]"
    fi
    export "FE2E_RELEASE_$s=$arr" "FE2E_ACCEPT_$s=$(sh_acc "$s")"
  done
}

# The flows the agreement must hold, one line per direction: <from> <to> <sorted release>.
sh_expected_flows() {
  local s them
  for s in "$A" "$B"; do
    them="$(other "$s")"
    sh_flows "$s" "$them" || continue
    echo "$s $them $(for x in $(sh_rel "$s"); do echo "$x/**"; done | LC_ALL=C sort | tr '\n' ',' | sed 's/,$//')"
  done
}

sh_digest() { echo "sha256:$(sha256sum "$WORK/$A/agreement-$B.json" 2>/dev/null | cut -d' ' -f1)"; }

shapes_main() { # $1 tag (h1|h2), and the scenario ids: $2 for A's view, $3 for B's view
  local n="$1" scA="$2" scB="$3" first="$A" second="$B" s them i set nonce
  set_partners "$A" "$B"; set_partners "$B" "$A"; NODIR=""
  sh_export_shape
  export_helpers
  setup_hosts "$n"
  i="$(iface "$first" "$second")"
  prepare_edge "$first" "$second" true
  prepare_edge "$second" "$first" false
  for s in "$A" "$B"; do cp "$WORK/$s/federation.toml" "$RECORD/s$n-$s-federation.toml"; done

  for s in "$first" "$second"; do
    them="$(other "$s")"
    rec "s$n-$s-$them-phases-1-to-5" --expect-in "exchange through the directory" -- bash -c "drive '$s' '$them' '$AVENUE' 1 5"
  done
  exchange "$first" "$second" "$AVENUE" "$n"
  for s in "$first" "$second"; do
    them="$(other "$s")"
    rec "s$n-$s-$them-phases-6-to-10" --expect-in "review what apply rendered" -- bash -c "drive '$s' '$them' '$AVENUE' 6 10"
  done
  rec "s$n-$first-$second-both-sides-hold-one-agreement" -- bash -c \
    "a=\$(docker exec $HP-$first-edge sha256sum /var/lib/goat-federation/$second/agreement.json | cut -d' ' -f1); b=\$(docker exec $HP-$second-edge sha256sum /var/lib/goat-federation/$first/agreement.json | cut -d' ' -f1); echo \"\$a \$b\"; [ -n \"\$a\" ] && [ \"\$a\" = \"\$b\" ]"
  sh_expected_flows > "$WORK/expected-flows.txt"
  for s in "$first" "$second"; do
    them="$(other "$s")"
    docker cp -q "$HP-$s-edge:/var/lib/goat-federation/$them/agreement.json" "$WORK/$s/agreement-$them.json" 2>/dev/null || true
    rec "s$n-$s-$them-flows-match-the-shape" -- python3 - "$WORK/$s/agreement-$them.json" "$WORK/expected-flows.txt" <<'PY'
import json, sys
try:
    ag = json.load(open(sys.argv[1]))
except Exception as e:
    print(f"no agreement to read: {e}"); sys.exit(1)
want = sorted(l.split() for l in open(sys.argv[2]).read().splitlines() if l.strip())
got = []
for f in ag.get("flows", []):
    to = f.get("to") or "".join(p["sovereign_id"] for p in ag.get("parties", []) if p["sovereign_id"] != f.get("from"))
    got.append([f.get("from"), to, ",".join(sorted(f.get("release", [])))])
got.sort()
print("declared:", want); print("composed:", got)
print("flows:", json.dumps(ag.get("flows", []), indent=1))
sys.exit(0 if got == want else 1)
PY
  done
  [ -s "$WORK/$A/agreement-$B.json" ] || die "no agreement composed; nothing after phase 10 can run"
  review_and_admit "$first" "$second"
  review_and_admit "$second" "$first"
  local digest; digest="$(sh_digest)"; echo "$digest" > "$RECORD/s$n-agreement-digest.txt"

  for s in "$first" "$second"; do
    them="$(other "$s")"
    if [ -n "$(sh_rel "$s")" ]; then
      rec "s$n-$s-$them-phases-11-to-12" "${EDGE_REFUSALS[@]}" --expect-in "open the wireguard transport" --expect-in "goat-fed-republisher-$them is running" -- bash -c "drive '$s' '$them' '$AVENUE' 11 12"
      local want=() x
      for x in $(sh_all_sets); do
        if sh_offers "$s" "$x"; then want+=(--expect-in "release $x/** -> wire from/$s/${x#release/}/**"); else want+=(--expect-not-in "release $x/**"); fi
      done
      rec "s$n-$s-$them-republisher-maps-the-offered-sets" "${want[@]}" -- docker exec "$HP-$s-core" docker logs "goat-fed-republisher-$them"
    else
      rec "s$n-$s-$them-phases-11-to-12" "${EDGE_REFUSALS[@]}" --expect-in "open the wireguard transport" -- bash -c "drive '$s' '$them' '$AVENUE' 11 12"
      # Evidence of what the role does on a side that releases nothing: deployed or not.
      rec "s$n-$s-$them-republisher-when-nothing-leaves" -- bash -c \
        "l=\$(ls '$RECORD'/*-s$n-$s-$them-phases-11-to-12.log 2>/dev/null | tail -1); grep -h -A3 'nothing leaves under this agreement' \"\$l\" | head -8; c=\$(docker exec $HP-$s-core docker ps -a --format '{{.Names}}' | grep goat-fed-republisher- || true); echo \"republisher containers on $s core: \${c:-none}\"; [ -z \"\$c\" ]"
    fi
  done
  rec "s$n-$first-$second-a-wireguard-handshake" -- bash -c \
    "for _ in \$(seq 1 30); do t=\$(docker exec $HP-$first-edge wg show $i latest-handshakes | cut -f2); [ \"\${t:-0}\" != 0 ] && break; sleep 2; done; docker exec $HP-$first-edge wg show $i latest-handshakes | cut -f2; [ \"\${t:-0}\" != 0 ]"
  for s in "$first" "$second"; do
    them="$(other "$s")"
    rec "s$n-$s-$them-phase-13" --expect-in "federate the SPIRE trust domains" -- bash -c "drive '$s' '$them' '$AVENUE' 13 13"
  done

  # Each offered set crosses in its own direction; every other set stays home. A direction
  # the agreement does not carry has no crossing check. Samples are staged at
  # release/<set>/shapes/<nonce>, two levels under the set as acceptance.sh stages them. Each
  # crossing waits up to SH_CROSS_WAIT seconds, staging again every 15, and says how long it
  # took. A key directly under the set (release/<set>/<nonce>) is recorded beside it as
  # evidence only (no catalog check reads it).
  for s in "$A" "$B"; do
    them="$(other "$s")"
    for set in $(sh_all_sets); do
      sh_offers "$s" "$set" && sh_flows "$s" "$them" || continue
      rec "s$n-$s-to-$them-${set#release/}-crosses" -- bash -c "sh_cross $s $them ${set#release/} ${SH_CROSS_WAIT:-300}"
      rec "s$n-$s-to-$them-${set#release/}-key-directly-under-the-set-crosses" --env SH_SUB= -- bash -c "sh_cross $s $them ${set#release/} 60"
    done
  done
  # A set staged where it is not offered must not cross. Where the direction carries a flow,
  # an offered control is staged beside it and must arrive, so silence means refusal and not
  # a dead path; where the agreement carries no flow that way, nothing may arrive at all.
  for s in "$A" "$B"; do
    them="$(other "$s")"
    local control=""; sh_flows "$s" "$them" && control="$(sh_rel "$s" | awk '{print $1}')"
    for set in $(sh_all_sets); do
      sh_offers "$s" "$set" && sh_flows "$s" "$them" && continue
      rec "s$n-$s-to-$them-${set#release/}-does-not-cross" -- bash -c "sh_stays $s $them ${set#release/} '${control#release/}'"
    done
  done
  sh_mark_absent "$n" "$scA" "$scB" "$digest"

  # Phase 14, as peer_up sets it up: a responder answering on one of its own offered sets,
  # and the three probes on each side.
  for s in "$first" "$second"; do
    them="$(other "$s")"
    set="$(sh_rel "$s" | awk '{print $1}')"
    if [ -n "$set" ]; then
      host_exec "$HP-$s-core" sh -c 'cat > /usr/local/bin/fed-responder.py' < "$SRC/ops/federation/roundtrip/responder.py"
      docker exec -d "$HP-$s-core" python3 /usr/local/bin/fed-responder.py "from/$them" "$set"
    fi
    ( start_probe "$s" "$them" agreed "/var/lib/goat-federation/$them/pki/peering-outward.pem" "/var/lib/goat-federation/$them/pki/peering-outward-key.pem" "$(probe_port "$s" "$them" 8101)" ) || echo "probe agreed on $s did not start" >> "$RECORD/s$n-probes.txt"
    ( start_probe "$s" "$them" forged /etc/fed-e2e/forged.pem /etc/fed-e2e/forged-key.pem "$(probe_port "$s" "$them" 8102)" ) || echo "probe forged on $s did not start" >> "$RECORD/s$n-probes.txt"
    ( start_probe "$s" "$them" impersonate "/etc/fed-e2e/impersonate-$them.pem" "/etc/fed-e2e/impersonate-$them-key.pem" "$(probe_port "$s" "$them" 8103)" ) || echo "probe impersonate on $s did not start" >> "$RECORD/s$n-probes.txt"
  done
  for s in "$first" "$second"; do
    them="$(other "$s")"
    rec "s$n-acceptance-as-$s-with-$them" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" --expect-not-in "ACCEPTANCE FAILED" \
      -- bash -c "accept_as '$s' '$them' '$AVENUE' 2>&1 | tee '$RECORD/s$n-acceptance-as-$s-with-$them.out'"
    # Whether acceptance refused the agreement for having one direction only.
    rec "s$n-acceptance-as-$s-with-$them-accepts-the-shape" --expect-not-in "offers no release set from" --expect-not-in "no round trip possible" -- \
      grep -E 'FAIL|skip|acceptance-counts' "$RECORD/s$n-acceptance-as-$s-with-$them.out"
  done
  collect "$n"
}

# The checks of a direction the agreement does not carry. $1 tag, $2 A's scenario, $3 B's.
sh_mark_absent() {
  local n="$1" scA="$2" scB="$3" digest="$4"
  if ! sh_flows "$A" "$B"; then
    mark "$scA.sends-cross" "NOT APPLICABLE" "agreement $digest offers no $A->$B flow"
    mark "$scB.receives-cross" "NOT APPLICABLE" "agreement $digest offers no $A->$B flow"
    mark "$scA.republisher-maps-sets" "NOT APPLICABLE" "agreement $digest offers no $A->$B flow"
  fi
  if ! sh_flows "$B" "$A"; then
    mark "$scA.receives-cross" "NOT APPLICABLE" "agreement $digest offers no $B->$A flow"
    mark "$scB.sends-cross" "NOT APPLICABLE" "agreement $digest offers no $B->$A flow"
    mark "$scB.republisher-maps-sets" "NOT APPLICABLE" "agreement $digest offers no $B->$A flow"
  fi
}
