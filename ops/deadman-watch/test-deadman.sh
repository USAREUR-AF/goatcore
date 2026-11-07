#!/usr/bin/env bash
# test-deadman.sh — the dead-man's switch judges each obs host on its own.
#
# Runs the real receiver on a loopback port and the real checker against a temp
# state directory, with no PagerDuty key, so a page shows up as the checker's
# "would have sent 'trigger'" log line and nothing leaves the machine.
#
# The case that matters is the F-529 one: two obs hosts post to one watcher, one
# of them goes quiet, and the other's heartbeat must NOT keep the switch green.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d)"
rpid=""
cleanup() { [ -n "$rpid" ] && kill "$rpid" 2>/dev/null; rm -rf "$tmp"; }
trap cleanup EXIT

fail() { echo "FAIL: $*"; exit 1; }

port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
DEADMAN_STATE_FILE="$tmp/last-heartbeat" DEADMAN_LISTEN_ADDR=127.0.0.1 \
  DEADMAN_LISTEN_PORT="$port" DEADMAN_TOKEN=t0ken \
  python3 "$here/deadman-receive.py" 2>"$tmp/receive.log" &
rpid=$!
for _ in $(seq 1 50); do
  curl -sf "http://127.0.0.1:$port/healthz" >/dev/null 2>&1 && break
  sleep 0.1
done

post() { curl -s --path-as-is -o /dev/null -w '%{http_code}' -X POST -H 'Authorization: Bearer t0ken' "http://127.0.0.1:$port$1"; }

# -- receiver ----------------------------------------------------------------
[ "$(post /obs-a)" = 200 ] || fail "POST /obs-a not accepted"
[ -f "$tmp/obs-a.last-heartbeat" ] || fail "POST /obs-a did not write obs-a.last-heartbeat"
[ "$(post /)" = 200 ] || fail "bare POST / not accepted"
[ -f "$tmp/last-heartbeat" ] || fail "bare POST / did not write the legacy file"
[ "$(post /../etc)" = 404 ] || fail "a path that is not a source name was accepted"
[ "$(post /obs.a)" = 404 ] || fail "a dotted source name was accepted"
[ "$(curl -s -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:$port/obs-a")" = 403 ] \
  || fail "a heartbeat without the token was accepted"
grep -q '127.0.0.1 "POST /obs-a' "$tmp/receive.log" || fail "receiver log does not name the peer and source"
echo "ok: receiver writes one file per source, keeps the legacy path, refuses bad names"

# -- checker -----------------------------------------------------------------
check() {
  DEADMAN_STATE_FILE="$tmp/last-heartbeat" DEADMAN_THRESHOLD_SECONDS=300 \
    DEADMAN_ROUTING_KEY="" DEADMAN_SOURCE=watcher DEADMAN_EXPECTED_SOURCES="$1" \
    bash "$here/deadman-check.sh" 2>&1
}
now=$(date +%s)

# F-529: obs-a is fresh, obs-b went quiet 10 minutes ago. Only obs-b pages.
echo "$now" > "$tmp/obs-a.last-heartbeat"
echo $(( now - 600 )) > "$tmp/obs-b.last-heartbeat"
out=$(check "obs-a obs-b")
echo "$out"
echo "$out" | grep -q "would have sent 'trigger': Alerting pipeline is DEAD on obs-b" \
  || fail "stale obs-b did not page while obs-a was fresh"
echo "$out" | grep -q "DEAD on obs-a" && fail "fresh obs-a paged"
echo "ok: a fresh obs host does not mask a stale one"

# An expected source that has never reported starts a clock, then pages.
rm -f "$tmp"/obs-c.*
out=$(check "obs-a obs-c")
echo "$out" | grep -q "obs-c: expected, no heartbeat yet — clock started" || fail "no clock started for obs-c"
echo "$out" | grep -q "DEAD on obs-c" && fail "obs-c paged on its first expected check"
echo $(( now - 600 )) > "$tmp/obs-c.expected-since"
out=$(check "obs-a obs-c")
echo "$out" | grep -q "would have sent 'trigger': Alerting pipeline is DEAD on obs-c" \
  || fail "an expected source that never reported stayed silent past the threshold"
echo "ok: a source that never reports pages once the threshold passes"

# Legacy mode (no expected list): single file, missing file is not death.
rm -f "$tmp/last-heartbeat"
out=$(check "")
echo "$out" | grep -q "no heartbeat recorded yet" || fail "legacy mode paged on a missing file"
echo $(( now - 600 )) > "$tmp/last-heartbeat"
out=$(check "")
echo "$out" | grep -q "would have sent 'trigger'" || fail "legacy mode did not page on a stale file"
echo "ok: legacy single-file mode unchanged"

echo "PASS: deadman-watch per-source judgement"
