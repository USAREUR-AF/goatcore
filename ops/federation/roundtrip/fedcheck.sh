#!/usr/bin/env bash
# Federation round-trip check — the final validation stage.
#
# STAGES a nonce under one of OUR release sets and waits for the counterparty's
# acknowledgement to come back under THEIR wire name. Our republisher is the only
# writer of our wire name, so the nonce crosses only if it lifted
# `release/<set>/fedcheck/<nonce>` onto `from/<us>/<set>/fedcheck/<nonce>`. A pass
# means the republisher ran, the sample crossed the peering, was delivered by the
# PARTNER'S OWN ROUTER to a process on their side, and the answer returned across
# the same agreement.
#
# Until an acknowledgement arrives it stages a probe with a FRESH nonce, the first
# resend one gap after the first probe and each later one twice as long after the
# last, up to 4 s (never closer than the gap), and an acknowledgement of any probe
# it sent completes the round trip. The receiving edge's downsampling drops a second sample inside its
# interval with nothing logged; when both sides run at once, our probe can arrive
# just behind their responder's acknowledgement of their own probe and be dropped.
# The nonce is fresh each time because the responder answers a key once only.
#
# This is the only check the SENDING side can make alone. Every other federation
# assertion requires shell on both sides, which no real publisher has.
#
#   fedcheck.sh <release/our-set> <from/their-wire> [timeout seconds, 20] [gap seconds, 1]
set -uo pipefail
BASE=${BASE:-http://127.0.0.1:8000}
OUR_SET=${1:?one of our release sets, staging name without the tail, e.g. release/weather}
THEIR_WIRE=${2:?their wire prefix, e.g. from/xaa}
case "$OUR_SET" in
  release/*) ;;
  *) echo "  FAIL  $OUR_SET is not a release/<set> staging name; our republisher is the only writer of our wire name"; exit 2 ;;
esac
TIMEOUT=${3:-20}
# The least time between two probes: the agreement's rate for one sender
# (acceptance.sh passes its own sample gap). Unknown, one second.
GAP=${4:-1}
for v in "$TIMEOUT" "$GAP"; do
  case "$v" in
    ''|.|*[!0-9.]*|*.*.*) echo "  FAIL  '$v' is not a number of seconds"; exit 2 ;;
  esac
done

now() { python3 -c 'import time;print(time.time())'; }
digest() {
  if command -v shasum >/dev/null 2>&1; then printf '%s' "$1" | shasum -a 256 | cut -d' ' -f1
  else printf '%s' "$1" | sha256sum | cut -d' ' -f1; fi
}

# Acknowledgements are polled every half second. The first resend is due one gap
# (at least one poll) after the first probe was sent, by the clock, so two probes are
# never closer than the gap; each later resend waits twice as long as the one before,
# up to CAP seconds (4, or the gap when it is longer). A 25 s wait at a half-second
# gap sends 9 probes.
POLL=0.5
EVERY=$(python3 -c "print(max($GAP, $POLL))")
CAP=$(python3 -c "print(max($GAP, 4))")
DEADLINE=$(python3 -c "import time;print(time.time() + $TIMEOUT)")
RUN="rt$(date +%s)$$r${RANDOM}"

SENT=""        # "<nonce>=<epoch seconds>" per probe our core accepted, in order
n=0; accepted=0; unanswered=0

# left: whole seconds before the deadline, at least 1, for a curl's own timeout, so
# no request outlives the deadline by more than a second. 0 once it has passed.
left() { python3 -c "import math,time; d=$DEADLINE-time.time(); print(0 if d <= 0 else max(1, math.ceil(d)))"; }

# send: stage one probe with a fresh nonce. While no probe has been staged, a core
# that answers anything but 2xx refused it, and the check fails at once. After one
# was staged, a refusal (a core under load) is counted like a PUT that got no
# answer, and the check goes on waiting for an acknowledgement.
send() {
  local nonce code t sent_at
  n=$((n + 1))
  nonce="${RUN}n${n}"
  t=$(left); [ "$t" -gt 8 ] && t=8; [ "$t" -ge 1 ] || t=1
  sent_at=$(now); LAST=$sent_at
  code=$(curl -s -m "$t" -o /dev/null -w '%{http_code}' -X PUT -d "fedcheck-$nonce" "$BASE/$OUR_SET/fedcheck/$nonce")
  case "$code" in
    2??) accepted=$((accepted + 1)); SENT="$SENT $nonce=$sent_at" ;;
    000|'') unanswered=$((unanswered + 1)) ;;
    *) if [ "$accepted" -eq 0 ]; then
         echo "  FAIL  our core refused the probe with HTTP $code ($BASE/$OUR_SET/fedcheck/$nonce); nothing was staged"
         exit 1
       fi
       unanswered=$((unanswered + 1)) ;;
  esac
}

# check: is any probe acknowledged? Sets HIT, HIT_AT and GOT. Each read's timeout is
# what is left before the deadline (at most 5 s), and the pass stops at the deadline.
check() {
  local e nonce r v t
  for e in $SENT; do
    nonce=${e%%=*}
    t=$(left); [ "$t" -ge 1 ] || return 1
    [ "$t" -gt 5 ] && t=5
    # `**`: the ack arrives under whichever of their sets they answer on.
    r=$(curl -s -m "$t" "$BASE/$THEIR_WIRE/**/fedcheck-ack/$nonce" 2>/dev/null)
    v=$(printf '%s' "$r" | grep -oE '"value":"[^"]*"' | head -1 | sed 's/.*"value":"//;s/"$//')
    if [ -n "$v" ]; then
      HIT=$nonce; HIT_AT=${e#*=}; GOT=$(printf '%s' "$v" | base64 -d 2>/dev/null)
      return 0
    fi
  done
  return 1
}

HIT=""; HIT_AT=""; GOT=""
LAST=""; every=$EVERY
send
while :; do
  if check; then break; fi
  [ "$(left)" -ge 1 ] || break
  if python3 -c "import sys,time; sys.exit(0 if time.time() - $LAST >= $every else 1)"; then
    send
    every=$(python3 -c "print(min($every * 2, $CAP))")
  fi
  sleep "$POLL"
done
t1=$(now)

if [ -z "$HIT" ]; then
  if [ "$accepted" -eq 0 ]; then
    echo "  FAIL  our core did not answer the probe PUT ($BASE, $unanswered tries in ${TIMEOUT}s)"
    exit 1
  fi
  echo "  FAIL  no acknowledgement in ${TIMEOUT}s — the sample did not complete the round trip"
  echo "        ($n probes sent, at least ${GAP}s apart; $unanswered not accepted by our core)"
  echo "        (a one-way check would have passed here if their router never delivered it)"
  exit 1
fi
i=${HIT##*n}   # the nonce ends n<its probe number>
rtt=$(python3 -c "print(f'{($t1-$HIT_AT)*1000:.0f}')")
saw=$(printf '%s' "$GOT" | python3 -c 'import json,sys; print(json.load(sys.stdin)["sha256"])' 2>/dev/null)
key=$(printf '%s' "$GOT" | python3 -c 'import json,sys; print(json.load(sys.stdin)["observed_key"])' 2>/dev/null)
want=$(digest "fedcheck-$HIT")
if [ "$saw" != "$want" ]; then
  echo "  FAIL  acknowledged, but the payload digest differs — sent $want, they saw $saw"
  exit 1
fi
echo "  PASS  round trip complete in ${rtt}ms"
echo "        they observed $key with the digest we sent (probe $i of $n)"
