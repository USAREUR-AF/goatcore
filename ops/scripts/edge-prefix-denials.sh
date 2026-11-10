#!/usr/bin/env bash
# edge-prefix-denials.sh — read ACL decisions off a sovereign edge router.
#
# WHY THIS EXISTS. A partner publishing outside the agreed `from/<sov>/**` prefix
# is refused, and per F-177 that refusal is SILENT at both ends: they see nothing
# delivered, we see nothing arrive, and no log line at either end says why. Two
# organisations then debug a network problem that is a naming problem. This is
# the fastest way to tell the difference, at the terminal, one edge at a time.
#
# For the same question asked CONTINUOUSLY rather than by hand, this is the
# ad-hoc twin of the peering health domain: ops/peering-denials/pd-snapshot.py
# collects the same lines on a timer and goat-peering-health publishes them on
# ops/peering/health/<edge>. See docs/operations/peering-denial-observability.md.
#
# It needs `goat_zenoh_edge_acl_trace: true` on that edge (default false, because
# allows log at the same level and it costs a line per ACL decision). If the
# switch is off you get a clear message saying so rather than an empty result
# that looks like "no denials".
#
# TWO ABSENCES, NEVER THE SAME ANSWER. "the partner is being refused" and "the
# partner never sent anything" both end with no data arriving. They are told
# apart by the ALLOW side of the same log, which is why this counts allows too
# and refuses to report a bare "no denials" when there was no traffic at all.
#
#   usage: edge-prefix-denials.sh <edge-container> [since]
#          edge-prefix-denials.sh edge-router 30m
set -uo pipefail

C="${1:-}"; SINCE="${2:-1h}"
if [ -z "$C" ]; then
  echo "usage: $0 <edge-container> [since]   e.g. $0 edge-router 30m" >&2
  exit 2
fi

if ! docker inspect "$C" >/dev/null 2>&1; then
  echo "no such container: $C" >&2
  exit 2
fi

# An empty result means nothing when the switch is off, so say which it is.
if ! docker inspect "$C" --format '{{range .Config.Env}}{{println .}}{{end}}' 2>/dev/null \
     | grep -q 'access_control=trace'; then
  cat >&2 <<EOF
refused: the prefix observer is OFF on $C.

Without RUST_LOG raising the ACL interceptor to trace, zenoh emits no denial
line at all, so "no denials found" would be indistinguishable from "no denials
happened". Set goat_zenoh_edge_acl_trace: true for this edge, re-deploy, and
reproduce.
EOF
  exit 3
fi

LOG=$(docker logs --since "$SINCE" "$C" 2>&1) || { echo "could not read logs for $C" >&2; exit 2; }
DENIED=$(grep -F 'is unauthorized to' <<<"$LOG" || true)
ALLOWED=$(grep -cF 'is authorized to' <<<"$LOG" || true)

if [ -z "$DENIED" ]; then
  if [ "${ALLOWED:-0}" -eq 0 ]; then
    echo "IDLE: no ACL decisions at all on $C in the last $SINCE — allowed or denied."
    echo "     The observer is on, so this is not blindness: nothing was ASKED of this"
    echo "     edge in the window. That is a different state from 'nothing was refused',"
    echo "     and the remedy is different too — look at the tunnel and the partner's"
    echo "     session before looking at the prefix."
    exit 0
  fi
  echo "ok: $ALLOWED ACL decisions on $C in the last $SINCE, none denied."
  echo "     If a partner reports sending data and nothing arriving, the traffic is"
  echo "     not reaching this edge at all — check the tunnel and the listener"
  echo "     before suspecting the prefix."
  exit 0
fi

echo "DENIALS on $C in the last $SINCE (against $ALLOWED allowed decisions):"
echo
# TWO LINE SHAPES, verified in the pinned zenoh =1.9.0 source rather than
# inferred from one sample (zenoh/src/net/routing/interceptor/access_control.rs):
#
#   evaluated (action(), :998)          "<zid> on <subject> is unauthorized to <msg> on <key>"
#   cached    (cached_result_or_action, :84/:393)
#                                        "Using cached result: <zid> is unauthorized to <msg> on <key>"
#
# Both are printed PER MESSAGE — the cache short-circuits the decision, not the
# log — so a repeated denial does keep producing lines. The cached shape carries
# no subject, which is why the SUBJECT column can read "-" on a busy link while
# the key expression, the diagnosis, is always present.
#
# <msg> is NOT a bare verb: it is "Put (ingress)", "Declare/Undeclare Subscriber
# (egress)" and so on (access_control.rs:879-901). An earlier version of this
# script matched it with `[a-z_]+`, which matches none of those — every line
# fell through the substitution unchanged and the table printed raw log lines in
# the KEY EXPRESSION column. A key expression cannot contain a space, which is
# what makes the trailing field safe to anchor on.
#
# uniq -c prefixes a count with leading spaces; fold it into a real field with
# sed rather than trying to strip it inside awk (POSIX awk has no regex
# backreferences, so the obvious sub() silently emits a literal backslash-one).
sed -E \
  -e 's/^.*Using cached result: ([^ ]+) is unauthorized to (.+) on ([^ ]+)[[:space:]]*$/\3|\2|-/' \
  -e 's/^.*[[:space:]]([0-9A-Za-z]+) on (.+) is unauthorized to (.+) on ([^ ]+)[[:space:]]*$/\4|\3|\2/' \
  <<<"$DENIED" \
  | sort | uniq -c | sort -rn | head -20 \
  | sed -E 's/^[[:space:]]*([0-9]+)[[:space:]]+/\1|/' \
  | awk -F'|' 'BEGIN{printf "  %-7s %-40s %-22s %s\n","COUNT","KEY EXPRESSION","ACTION","SUBJECT"}
       {printf "  %-7s %-40s %-22s %s\n", $1, $2, $3, $4}'
echo
echo "  The KEY EXPRESSION column is the diagnosis: it is the prefix the partner is"
echo "  actually publishing on. Compare it to the from/<sov>/** agreed for this"
echo "  peering. If they differ, this is a naming mismatch, not a network fault."
echo "  A COUNT above 1 is the refusal REPEATING — the partner is still trying."
