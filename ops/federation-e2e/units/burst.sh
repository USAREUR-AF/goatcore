#!/usr/bin/env bash
# units/burst.sh — a lane's pacing measured at the receiving edge (ADR 1244): a peering up
# through phase 14 on role cores (units/pair-role.sh's scenario), then a burst of samples
# released about 1 ms apart on one lane. The receiving edge passes at most one message per
# 1/hz on a lane and drops the rest, so a sender that paces loses nothing and one that does
# not loses most of the burst. The receiving edge's own log says which: its ACL allow for
# each sample that crossed, its downsampling trace for each one dropped. The receiving core
# must store every sample, and the sending republisher must count no queue_full drop.
# Then, unless FE2E_BURST_OVERFLOW=0, the same with the sender's queue lowered to
# FE2E_BURST_DEPTH (8): what does not fit is counted as queue_full and warned about, and
# nothing reaches the edge too close to be dropped there.
# Sourced by lib/unit.sh. Scenario: lane-pacing-burst.
#
#   FE2E_BURST_COUNT    samples per burst (default 50)
#   FE2E_BURST_GAP_MS   the release spacing (default 1)
#   FE2E_BURST_DEPTH    the sender's queue depth for the overflow burst (default 8)
#   FE2E_BURST_OVERFLOW 0 skips the overflow burst
# shellcheck source=/dev/null
. "$HERE/units/pair-role.sh"

BU_COUNT="${FE2E_BURST_COUNT:-50}"
BU_GAP_MS="${FE2E_BURST_GAP_MS:-1}"
BU_DEPTH="${FE2E_BURST_DEPTH:-8}"

# The receiving edge's compose file, its config, and the sender's republisher compose file.
bu_edge_compose() { echo "/opt/goat-zenoh-edge-$1/docker-compose.yml"; } # $1 sender (the edge's partner)
bu_rp_compose() { echo "/opt/goat-fed-republisher-$1/docker-compose.yml"; } # $1 receiver
bu_queue_full() { # $1 sender, $2 receiver: the sender's queue_full drops so far
  docker exec "$HP-$1-core" curl -s -m 5 "http://127.0.0.1:$(rp_metrics "$1" "$2")/metrics" | python3 "$HERE/lib/burst.py" metric
}

# The receiving edge's router with its downsampling module at trace as well as its ACL, so
# each drop is named. The role renders RUST_LOG only with goat_zenoh_edge_acl_trace (the
# lab's overlay sets it); the edge is recreated, so the link is shown back afterwards.
bu_edge_trace() { # $1 receiver (the edge's host), $2 sender
  local h="$HP-$1-edge" f; f="$(bu_edge_compose "$2")"
  docker exec "$h" grep -q 'RUST_LOG:.*access_control=trace' "$f" || { echo "$f has no ACL trace RUST_LOG line"; return 1; }
  docker exec "$h" sed -i '/RUST_LOG:/{/downsampling=trace/!s/access_control=trace/access_control=trace,zenoh::net::routing::interceptor::downsampling=trace/}' "$f"
  docker exec "$h" grep 'RUST_LOG:' "$f"
  docker exec "$h" docker compose -f "$f" up -d 2>&1 | tail -3
  docker exec "$h" docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "goat-zenoh-edge-$2" | grep -q 'downsampling=trace'
}

# The sender's republisher with another queue depth; it is recreated, so the link is shown
# back afterwards. $1 sender, $2 receiver, $3 depth.
bu_depth() {
  local h="$HP-$1-core" f; f="$(bu_rp_compose "$2")"
  docker exec "$h" sed -i -E "s/(FED_REPUBLISHER_QUEUE_DEPTH: )\"[0-9]+\"/\\1\"$3\"/" "$f"
  docker exec "$h" docker compose -f "$f" up -d 2>&1 | tail -3
  docker exec "$h" docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "goat-fed-republisher-$2" | grep -qx "FED_REPUBLISHER_QUEUE_DEPTH=$3"
}
export -f bu_edge_compose bu_rp_compose bu_queue_full bu_edge_trace bu_depth

# One burst, then the receiving edge's log and the receiving core's store read. $1 label,
# $2 sender, $3 receiver. Leaves $RECORD/burst-<label>.{sent,edge.json,log,stored,qf}.
bu_burst() {
  local l="$1" s="$2" them="$3" p before
  p="fe2ebst$l$(date +%s)"
  before="$(bu_queue_full "$s" "$them")"
  : > "$RECORD/burst-$l.sent"
  python3 "$HERE/lib/burst.py" send --base "http://$(core_ip "$s"):8000" --prefix "release/all/$p" \
    --count "$BU_COUNT" --spacing-ms "$BU_GAP_MS" --out "$RECORD/burst-$l.sent"
  # The paced burst drains at 1.1/hz a sample; at 1 Hz, 50 take a minute.
  sleep "$(python3 -c 'import sys; print(int(5 + 1.1 * int(sys.argv[1]) / float(sys.argv[2])))' "$BU_COUNT" "$BU_HZ")"
  printf '%s %s\n' "$before" "$(bu_queue_full "$s" "$them")" > "$RECORD/burst-$l.qf"
  docker exec "$HP-$them-edge" docker logs -t "goat-zenoh-edge-$s" > "$RECORD/burst-$l.log" 2>&1 || true
  python3 "$HERE/lib/burst.py" edge --log "$RECORD/burst-$l.log" --key-prefix "from/$s/all/$p/" > "$RECORD/burst-$l.edge.json"
  stored_at "$them" "from/$s/all/$p/**" | grep -c '^b[0-9]*$' > "$RECORD/burst-$l.stored" || true
  docker exec "$HP-$s-core" docker logs --since 5m "goat-fed-republisher-$them" 2>&1 | grep -c 'paced queue to the peering edge is full' > "$RECORD/burst-$l.warned" || true
}

bu_judge() { # $1 label, then judge flags
  local l="$1" qf; shift; read -r -a qf < "$RECORD/burst-$l.qf"
  python3 "$HERE/lib/burst.py" judge --sent "$RECORD/burst-$l.sent" --edge "$RECORD/burst-$l.edge.json" --hz "$BU_HZ" \
    --stored "$(cat "$RECORD/burst-$l.stored")" --queue-full-before "${qf[0]}" --queue-full-after "${qf[1]}" "$@"
}
export -f bu_judge

unit_main() {
  FE2E_CORE=role; export FE2E_CORE
  local n first second; n="$(role_n)"; first="$(role_first)"; second="$(other "$first")"
  scenario "$n" "$first" "$AVENUE"
  docker cp -q "$HP-$second-edge:/etc/goat-zenoh-edge-$first/config.json5" "$RECORD/burst-edge-config.json5"
  rec "bu-lane-rate-read" -- python3 "$HERE/lib/burst.py" hz --config "$RECORD/burst-edge-config.json5" --key "from/$first/**"
  BU_HZ="$(python3 "$HERE/lib/burst.py" hz --config "$RECORD/burst-edge-config.json5" --key "from/$first/**" 2>/dev/null || echo 0)"
  export BU_HZ
  if [ "$BU_HZ" = 0 ]; then
    for c in edge-trace link-back paced; do
      mark "lane-pacing-burst.$c" BLOCKED "the receiving edge's config holds no downsampling rule for from/$first/**"
    done
    return 0
  fi
  rec "bu-$second-edge-traces-downsampling" -- bash -c "bu_edge_trace $second $first"
  rec "bu-link-back-after-the-edge-restart" -- bash -c "round_trip_eventually $first $second"

  bu_burst paced "$first" "$second"
  rec "bu-paced-burst-crosses-spaced-and-stored" -- bash -c "bu_judge paced"

  if [ "${FE2E_BURST_OVERFLOW:-1}" = 0 ]; then
    for c in queue-lowered link-back-after-queue overflow queue-restored; do mark "lane-pacing-burst.$c" SKIP "FE2E_BURST_OVERFLOW=0"; done
    return 0
  fi
  rec "bu-sender-queue-lowered" -- bash -c "bu_depth $first $second $BU_DEPTH"
  rec "bu-link-back-after-the-queue-change" -- bash -c "round_trip_eventually $first $second"
  bu_burst overflow "$first" "$second"
  rec "bu-overflow-counted-and-warned" -- bash -c "bu_judge overflow --overflow --warned \$(cat $RECORD/burst-overflow.warned)"
  rec "bu-sender-queue-restored" -- bash -c "bu_depth $first $second 4096"
}
