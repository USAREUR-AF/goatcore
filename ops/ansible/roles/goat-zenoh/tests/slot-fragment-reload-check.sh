#!/usr/bin/env bash
# Checks files/slot-fragment-reload.py (the router host's reload watcher,
# GHSA-rr3p) against a dummy container standing in for the router: the
# watcher records a fragment as loaded ONLY when the restart command succeeds
# and actually replaced a running container; a failed, no-op or router-killing
# restart leaves the record where it was (so the portal keeps answering
# "pending") and the next run retries. Also: prompt revocations are throttled
# to --prompt-burst per --interval.
#
# Requires docker and a local alpine image. Every name carries
# ${RR3P_PROOF_PREFIX:-rr3p}-reload.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
WATCHER="$HERE/../files/slot-fragment-reload.py"
P="${RR3P_PROOF_PREFIX:-rr3p}-reload"
C="$P-router"
WORK="$(mktemp -d -t "$P.XXXXXX")"
trap 'docker rm -f "$C" >/dev/null 2>&1; rm -rf "$WORK"' EXIT
CFG="$WORK/config.json5"; LOADED="$WORK/slot-fragment.loaded.json5"
A0=00000000000000000000000000000000; A1=11111111111111111111111111111111
FAILS=0
ok()  { echo "  PASS: $*"; }
bad() { echo "  FAIL: $*"; FAILS=$((FAILS+1)); }

put_config() { # <cn0|-> <cn1|-> [copub0: live|ended]: the portal's atomic write
  python3 - "$HERE/slot-fragment" "$A0" "$1" "$A1" "$2" "${3:-}" > "$CFG.tmp" <<'PY'
import sys
sys.path.insert(0, sys.argv[1])
import slotfragment as sf
a0, c0, a1, c1, cp = sys.argv[2:7]
copub = [("multipub/pool/t1", cp == "live")] if cp else []
frag = sf.render([(a0, "" if c0 == "-" else c0, copub), (a1, "" if c1 == "-" else c1)])
print("access_control: {\n  subjects: [\n      // BEGIN: slot subjects\n" + frag + "      // END: slot subjects\n  ],\n}")
PY
  mv "$CFG.tmp" "$CFG"
}
RECREATE="docker rm -f $C >/dev/null 2>&1; docker run -d --name $C alpine:latest sleep 3600 >/dev/null"
watch() { # <restart> [args]: prints output, returns exit code
  python3 "$WATCHER" --config "$CFG" --container "$C" --settle 1 --min-gap 0 --interval 600 --restart "$1" "${@:2}" 2>&1
}
age_record() { touch -d '-30 seconds' "$LOADED" 2>/dev/null; }
loaded_has() { grep -q "\"$1\"" "$LOADED" 2>/dev/null; }

docker run -d --name "$C" alpine:latest sleep 3600 >/dev/null || { echo "cannot start $C (needs alpine:latest)"; exit 2; }

echo "== first load"
put_config "$A0.g1" -
out="$(watch "$RECREATE")"; rc=$?; echo "$out" | sed 's/^/    /'
[ $rc -eq 0 ] && loaded_has "$A0.g1" && ok "a real recreate is recorded" || bad "first load not recorded (rc=$rc)"

echo "== revoke; the restart command fails and the old container keeps running"
put_config - -; age_record
out="$(watch 'echo "compose: simulated failure" >&2; false')"; rc=$?; echo "$out" | sed 's/^/    /'
[ $rc -ne 0 ] && loaded_has "$A0.g1" && ok "failed restart: exit $rc, record not advanced (portal keeps saying pending)" \
  || bad "a failed restart was recorded as loaded"
out="$(watch true)"; rc=$?; echo "$out" | sed 's/^/    /'
[ $rc -ne 0 ] && loaded_has "$A0.g1" && ok "no-op restart (exit 0, same container): exit $rc, record not advanced" \
  || bad "a no-op restart was recorded as loaded"
out="$(watch "docker stop -t 0 $C >/dev/null")"; rc=$?; echo "$out" | sed 's/^/    /'
[ $rc -ne 0 ] && loaded_has "$A0.g1" && ok "restart left the router down: exit $rc, record not advanced" \
  || bad "a stopped router was recorded as loaded"
docker start "$C" >/dev/null
out="$(watch "$RECREATE")"; rc=$?; echo "$out" | sed 's/^/    /'
[ $rc -eq 0 ] && ! loaded_has "$A0.g1" && ok "the retry with a working restart loads the revocation" \
  || bad "the retry did not load the revocation (rc=$rc)"

echo "== prompt revocations are throttled to --prompt-burst per --interval"
rm -f "$WORK/slot-fragment.reloads"
put_config "$A0.g2" -; touch -d '-700 seconds' "$LOADED"   # an addition, its batch window passed
watch "$RECREATE" --prompt-burst 2 >/dev/null
put_config "$A0.g3" -; age_record                            # a removal: prompt (2nd recreate in the window)
watch "$RECREATE" --prompt-burst 2 >/dev/null
loaded_has "$A0.g3" || bad "setup: the second recreate did not load"
put_config - -; age_record
out="$(watch "$RECREATE" --prompt-burst 2)"; rc=$?; echo "$out" | sed 's/^/    /'
echo "$out" | grep -q 'throttled' && loaded_has "$A0.g3" && ok "a third removal inside the window waits for the interval" \
  || bad "removals were not throttled"

echo "== a co-publish grant ending while the slot keeps its holder is a revocation (prompt, not batched)"
rm -f "$WORK/slot-fragment.reloads"
put_config "$A0.g5" - live; touch -d '-700 seconds' "$LOADED"; watch "$RECREATE" >/dev/null
put_config "$A0.g5" - ended; age_record
out="$(python3 "$WATCHER" --config "$CFG" --container "$C" --settle 1 --min-gap 15 --interval 300 --restart "$RECREATE" 2>&1)"; rc=$?
echo "$out" | sed 's/^/    /'
copub_line="$(grep -A1 "id: \"copub-$A0-" "$LOADED" | tail -1)"
[ $rc -eq 0 ] && echo "$out" | grep -q '^reloaded' && echo "$out" | grep -q '1 removed' \
  && echo "$copub_line" | grep -q unassigned && grep -q "\"$A0.g5\"" "$LOADED" \
  && ok "the co-publish revoke loaded within min-gap although the CN stays on slot-<anchor>" \
  || bad "a co-publish revoke was batched or not loaded"

echo "== a backwards clock step does not hold a revocation"
rm -f "$WORK/slot-fragment.reloads"
put_config "$A0.g4" -; touch -d '-700 seconds' "$LOADED"; watch "$RECREATE" >/dev/null
put_config - -; touch -d '+3600 seconds' "$LOADED"               # the record is "newer" than the clock
echo "9999999999" > "$WORK/slot-fragment.reloads"                  # and so is a recreate stamp
out="$(python3 "$WATCHER" --config "$CFG" --container "$C" --settle 1 --min-gap 15 --interval 300 --restart "$RECREATE" 2>&1)"; rc=$?
echo "$out" | sed 's/^/    /'
[ $rc -eq 0 ] && ! loaded_has "$A0.g4" && ok "a record mtime in the future is treated as old: the revocation loads now" \
  || bad "a backwards clock step held the revocation"

echo
[ $FAILS -eq 0 ] && echo "SLOT-FRAGMENT RELOAD CHECKS PASSED" || echo "$FAILS CHECK(S) FAILED"
exit $FAILS
