#!/usr/bin/env bash
# lib/flow.sh — data keeps arriving while something happens (gate rows B3 and B4): a steady
# flow of fresh values from one sovereign to the other, each released once and read back at
# the receiver afterwards, so a step's effect on the flow is measured rather than assumed.
# Sourced by a unit after units/pair.sh (put_release, stored_at).
#
#   flow_start <sender> <receiver> <label> [reader]
#                                            one fresh value a second, in the background; with
#                                            `reader`, also a reader on the receiver's core that
#                                            records each sample's number and arrival time
#                                            (lib/samples.py)
#   flow_stop <label>                        stops both, waits, reads every value back
#   flow_report <label>                      totals, the longest run lost, the last stored
#
# Each sample is named fl<label><epoch>n<i>, and <i> counts up from 1 for the whole flow:
# the number the key-rotation scenario's samples-<key> check reads (federation-key-rotation.md
# §6). The reader is the receiver core's REST subscription (Server-Sent Events) on the
# sender's release keys; its records are in flow-<label>.arrived. It is a subscriber, so a
# unit that measures reception with nothing subscribed (gate row B4, units/flow.sh) starts
# its flow without one.
#
# A release with no route is dropped by design (docs/design/zenoh-brokerless-consequences.md,
# row 4), so what a check reads is how long and how many, against what its scenario allows.

flow_start() { # $1 sender, $2 receiver, $3 label, $4 "reader" for an arrival-recording reader
  local s="$1" them="$2" l="$3"
  : > "$RECORD/flow-$l.sent"
  : > "$RECORD/flow-$l.arrived"
  printf '%s %s\n' "$s" "$them" > "$RECORD/flow-$l.pair"
  rm -f "$RECORD/flow-$l.reader.pid"
  if [ "${4:-}" = reader ]; then
    # The reader first, so the first sample has someone to arrive at.
    (
      curl -sN -H 'Accept: text/event-stream' "http://$(core_ip "$them"):8000/from/$s/all/**" \
        | python3 "$HERE/lib/samples.py" record "$RECORD/flow-$l.arrived" --prefix "fl$l"
    ) > /dev/null 2>&1 &
    echo $! > "$RECORD/flow-$l.reader.pid"
    sleep 1
  fi
  (
    i=0
    while [ ! -e "$RECORD/flow-$l.stop" ]; do
      i=$((i + 1)); k="fl$l$(date +%s)n$i"
      code="$(put_release "$s" all "$them" "$k" 2>/dev/null || echo 000)"
      printf '%s\t%s\t%s\n' "$(date +%s.%N)" "$k" "$code" >> "$RECORD/flow-$l.sent"
      sleep 1
    done
  ) > /dev/null 2>&1 &
  echo $! > "$RECORD/flow-$l.pid"
}

flow_stop() { # $1 label
  local l="$1" s them t k code
  touch "$RECORD/flow-$l.stop"
  wait "$(cat "$RECORD/flow-$l.pid")" 2>/dev/null || true
  read -r s them < "$RECORD/flow-$l.pair"
  sleep 8 # the last values travel, and reach the reader
  if [ -e "$RECORD/flow-$l.reader.pid" ]; then
    # The reader's subshell, its curl and its recorder.
    pkill -P "$(cat "$RECORD/flow-$l.reader.pid")" 2>/dev/null || true
    kill "$(cat "$RECORD/flow-$l.reader.pid")" 2>/dev/null || true
  fi
  : > "$RECORD/flow-$l.read"
  while IFS=$'\t' read -r t k code; do
    printf '%s\t%s\t%s\t%s\n' "$t" "$k" "$code" "$([ "$(stored_at "$them" "from/$s/all/$k" | head -1)" = "$k" ] && echo stored || echo lost)" >> "$RECORD/flow-$l.read"
  done < "$RECORD/flow-$l.sent"
}

flow_report() { # $1 label -> one JSON line, and exit 0
  python3 - "$RECORD/flow-$1.read" <<'PY'
import json, sys
rows = [l.rstrip("\n").split("\t") for l in open(sys.argv[1]) if l.strip()]
t = [float(r[0]) for r in rows]; ok = [r[3] == "stored" for r in rows]
runs, cur = [], None
for i, o in enumerate(ok):
    if not o and cur is None: cur = i
    if o and cur is not None: runs.append((cur, i - 1)); cur = None
if cur is not None: runs.append((cur, len(ok) - 1))
long = max(runs, key=lambda r: r[1] - r[0], default=None)
gap = None
if long:
    end = t[long[1] + 1] if long[1] + 1 < len(t) else t[long[1]]
    gap = round(end - t[long[0]], 1)
print(json.dumps({"released": len(rows), "stored": sum(ok), "lost": len(ok) - sum(ok),
                  "runs_lost": len(runs), "longest_run_lost": (long[1] - long[0] + 1) if long else 0,
                  "longest_gap_s": gap or 0,
                  "first_stored": ok.index(True) + 1 if True in ok else None,
                  "last_five_stored": all(ok[-5:]) if len(ok) >= 5 else False}))
PY
}
export -f flow_start flow_stop flow_report
