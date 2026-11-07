#!/usr/bin/env bash
# deadman-check — the deciding half of the dead-man's switch.
#
# Reads the timestamp deadman-receive last wrote and pages if it has gone stale.
# Run from a systemd timer, separately from the receiver, so that the RECEIVER
# dying is also caught: no receiver means no heartbeats means a stale file.
#
# It pages PagerDuty's Events API DIRECTLY and deliberately. The whole premise is
# that the obs host may be dead, so anything routed through its Alertmanager is
# unavailable exactly when this fires. A dead-man's switch that reports through
# the system it monitors is decorative.
#
# Env (from the systemd unit):
#   DEADMAN_STATE_FILE        timestamp file written by deadman-receive
#   DEADMAN_THRESHOLD_SECONDS staleness that counts as death
#   DEADMAN_ROUTING_KEY       PagerDuty Events API v2 integration key
#   DEADMAN_SOURCE            what to name this watcher in the incident
#   DEADMAN_EXPECTED_SOURCES  space-separated obs hostnames, each checked on its own
#
# PER-SOURCE MODE. With DEADMAN_EXPECTED_SOURCES set, every listed obs
# host is judged on its own timestamp file (<state-dir>/<source>.last-heartbeat,
# written by deadman-receive for POST /<source>), with its own dedup key and its
# own paged marker. One healthy obs host can no longer keep the switch green for
# a dead one — which is how Orin-C was dead for four days in September 2026
# while the SVK obs host's heartbeat kept the single shared file fresh.
# Empty list = the original single-file behaviour, unchanged.
set -uo pipefail

STATE_FILE="${DEADMAN_STATE_FILE:-/var/lib/deadman-watch/last-heartbeat}"
THRESHOLD="${DEADMAN_THRESHOLD_SECONDS:-300}"
ROUTING_KEY="${DEADMAN_ROUTING_KEY:-}"
SOURCE="${DEADMAN_SOURCE:-$(hostname -s)}"
EXPECTED_SOURCES="${DEADMAN_EXPECTED_SOURCES:-}"
STATE_DIR="$(dirname "$STATE_FILE")"
# Endpoint from the environment, US only as a fallback. EU-hosted PagerDuty
# accounts MUST use events.eu.pagerduty.com — the same rule
# ops/monitoring/net-health/alert-eval.py already follows. This was hardcoded to
# the US endpoint, which meant an EU account would have had its dead-man's
# switch firing into the wrong region: accepted by the API, seen by nobody.
PD_URL="${DEADMAN_EVENTS_URL:-${PAGERDUTY_EVENTS_URL:-https://events.pagerduty.com/v2/enqueue}}"

log() { echo "deadman-check: $*" >&2; }

pd_event() {
  # $1 = trigger|resolve, $2 = summary, $3 = dedup key
  if [ -z "$ROUTING_KEY" ]; then
    log "NO ROUTING KEY — would have sent '$1': $2"
    return 1
  fi
  local payload
  payload=$(cat <<EOF
{"routing_key":"${ROUTING_KEY}","event_action":"$1","dedup_key":"$3",
 "payload":{"summary":"$2","source":"${SOURCE}","severity":"critical",
 "component":"observability","group":"dead-mans-switch"}}
EOF
)
  local code
  code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 15 \
    -X POST -H 'Content-Type: application/json' -d "$payload" "$PD_URL" 2>/dev/null)
  if [ "$code" = "202" ]; then
    log "PagerDuty accepted '$1' (dedup_key=$3)"
    return 0
  fi
  log "PagerDuty '$1' FAILED with HTTP ${code:-none}"
  return 1
}

# judge <state-file> <dedup-key> <label> <expected-since-file|"">
#
# One staleness verdict. <label> names what went quiet in the incident text.
# With an expected-since file, a heartbeat file that has NEVER appeared is timed
# from the first check that expected it; without one (legacy mode), a missing
# file is not death — see below.
judge() {
  local state="$1" dedup="$2" label="$3" since_file="$4"
  local marker="${state}.paged" last now age

  now=$(date +%s)
  if [ ! -f "$state" ]; then
    if [ -z "$since_file" ]; then
      # Legacy: a missing state file is NOT treated as death. On first boot the
      # receiver may not have taken a heartbeat yet, and paging then would train
      # everyone to ignore this alert on day one.
      log "no heartbeat recorded yet (${state} absent) — not paging; waiting for the first"
      return 0
    fi
    # Per-source: an expected source that never reports must not stay silent
    # forever — that is the masking this mode exists to end. Start its clock at
    # the first check that expected it.
    if [ ! -f "$since_file" ]; then
      echo "$now" > "$since_file"
      log "${label}: expected, no heartbeat yet — clock started"
      return 0
    fi
    last=$(tr -dc '0-9' < "$since_file" 2>/dev/null)
    [ -z "$last" ] && last=0
  else
    [ -n "$since_file" ] && rm -f "$since_file"
    last=$(tr -dc '0-9' < "$state" 2>/dev/null)
    if [ -z "$last" ]; then
      log "${label}: state file unreadable/empty — treating as STALE (an unparseable state must never read as healthy)"
      last=0
    fi
  fi

  age=$(( now - last ))

  if [ "$age" -gt "$THRESHOLD" ]; then
    if [ -f "$marker" ]; then
      log "${label}: still stale (${age}s > ${THRESHOLD}s); already paged, not re-sending"
      return 0
    fi
    log "${label}: STALE — last heartbeat ${age}s ago, threshold ${THRESHOLD}s. Paging."
    if pd_event trigger "Alerting pipeline is DEAD on ${label} — no heartbeat in ${age}s. Its Prometheus/Alertmanager cannot report anything, including its own failure." "$dedup"; then
      touch "$marker"
    fi
    return 0
  fi

  # Healthy. If we had paged, resolve it — a dead-man's switch that never clears
  # leaves a permanently-open incident, which is its own kind of noise.
  if [ -f "$marker" ]; then
    log "${label}: heartbeat returned (${age}s ago) — resolving"
    if pd_event resolve "Alerting pipeline heartbeat from ${label} has returned." "$dedup"; then
      rm -f "$marker"
    fi
  fi
  return 0
}

if [ -z "$EXPECTED_SOURCES" ]; then
  judge "$STATE_FILE" "deadman-${SOURCE}" "the observability host" ""
  exit 0
fi

for src in $EXPECTED_SOURCES; do
  if ! printf '%s' "$src" | grep -Eq '^[a-z0-9][a-z0-9-]{0,62}$'; then
    log "expected source '${src}' is not a valid source name — skipping it LOUDLY; fix DEADMAN_EXPECTED_SOURCES"
    continue
  fi
  judge "${STATE_DIR}/${src}.last-heartbeat" "deadman-${SOURCE}-${src}" "$src" "${STATE_DIR}/${src}.expected-since"
done

exit 0
