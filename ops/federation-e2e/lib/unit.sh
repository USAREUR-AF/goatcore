#!/usr/bin/env bash
# unit.sh — run one unit of the federation e2e environment in one lab.
#
#   lib/unit.sh <unit>     (called by ops/federation-e2e/env; not meant to be run by hand)
#
# A unit is ops/federation-e2e/units/<unit>.sh: it brings sovereigns up in its lab with the
# engine (lib/engine.sh) and records steps with rec. Several scenarios of the catalog
# (scenarios/catalog.yaml) read one unit's steps. The environment comes from env:
#
#   FE2E_LAB, FE2E_HOST_PREFIX, FE2E_NET_PREFIX, FE2E_NET_BASE, FE2E_PARTIES  (lib/engine.sh)
#   SRC       the goatcore tree under test        GOAT, GB  its built binaries
#   AVENUE    script | goatbuilder                WORK, RECORD  this unit's directories
#   KEEP      1 leaves the lab up                 FE2E_HOST_IMAGE, FE2E_RP_TAG, RP_IMAGE
#
# The unit writes into RECORD: report.jsonl (rec.py), marks.jsonl (mark), and stopped.json
# when it ends before its last step, so that verdict.py tells BLOCKED from SKIP.
set -euo pipefail
UNIT="$1"
HERE="$(cd "$(dirname "$0")/.." && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
: "${SRC:?}" "${GOAT:?}" "${GB:?}" "${AVENUE:?}" "${WORK:?}" "${RECORD:?}"
KEEP="${KEEP:-0}"; ONLY=""; PREPARE=0
mkdir -p "$WORK" "$RECORD"
export E2E_RECORD_DIR="$RECORD" GIT_LFS_SKIP_SMUDGE=1
ZENOH_IMAGE="ghcr.io/usareur-af/zenoh-pqc@sha256:372de39d2e8b64fe7157194173f1b7ac6eac2700ff04c49cce2ce3c284a6fdb2"
UNIT_DONE=0
unit_exit() {
  local rc=$?
  if [ "$UNIT_DONE" != 1 ]; then
    local why="exit $rc"
    [ -s "$RECORD/stopped.reason" ] && why="$(tail -1 "$RECORD/stopped.reason")"
    python3 -c 'import json,sys; print(json.dumps({"reason": sys.argv[1]}))' "$why" > "$RECORD/stopped.json"
  fi
  if declare -F final >/dev/null; then final; fi
}
trap unit_exit EXIT
# Stopped from outside: the reason says so, and env publishes nothing for an interrupted run.
killtree() { local c; for c in $(pgrep -P "$1" 2>/dev/null); do killtree "$c"; done; kill -TERM "$1" 2>/dev/null || true; }
UNIT_PID=""
on_signal() {
  grep -qs "timed out" "$RECORD/stopped.reason" || printf "%s\n" "terminated by a signal; the run was interrupted" >> "$RECORD/stopped.reason"
  [ -z "$UNIT_PID" ] || killtree "$UNIT_PID"
  exit 143
}
trap on_signal TERM INT HUP
# A unit that hangs stops at FE2E_UNIT_TIMEOUT seconds (default 7200); its later checks read
# BLOCKED with that reason, and the run goes on.
( sleep "${FE2E_UNIT_TIMEOUT:-7200}"; printf "%s\n" "timed out after ${FE2E_UNIT_TIMEOUT:-7200} s" >> "$RECORD/stopped.reason"; kill -TERM $$ 2>/dev/null ) >/dev/null 2>&1 &
UNIT_WATCHDOG=$!
# The engine starts RP_IMAGE empty (run.sh builds it); the environment built it already.
rp_given="${RP_IMAGE:-}"
# shellcheck source=engine.sh
. "$HERE/lib/engine.sh"
RP_IMAGE="${rp_given:?RP_IMAGE: the republisher image id}"

# die() also records why the unit stopped; every later step of it reads BLOCKED.
die() { echo "unit $UNIT: $*" >&2; printf '%s\n' "$*" >> "$RECORD/stopped.reason"; exit 1; }
# mark <scenario>.<check> BLOCKED|SKIP|"NOT APPLICABLE" <reason>: a check the unit decided
# not to run, or that the agreement excludes. A mark outranks a passing step of the same
# check, and never a failed one.
mark() {
  # PASS and FAIL come only from recorded steps (lib/verdict.py); a mark needs a reason.
  case "$2" in BLOCKED|SKIP|"NOT APPLICABLE") ;; *) echo "mark: a mark may say only BLOCKED, SKIP or NOT APPLICABLE, not '$2'" >&2; exit 2 ;; esac
  [ -n "${3:-}" ] || { echo "mark: $1 has no reason" >&2; exit 2; }
  python3 -c 'import json,sys; print(json.dumps({"check": sys.argv[1], "outcome": sys.argv[2], "reason": sys.argv[3]}))' "$1" "$2" "$3" >> "$RECORD/marks.jsonl"
}
# has_text <file-in-SRC> <text>: whether the tree under test carries a capability.
has_text() { grep -qF -- "$2" "$SRC/$1" 2>/dev/null; }
export -f mark has_text

# shellcheck source=/dev/null
. "$HERE/units/$UNIT.sh"
# In the background, so that a signal (the timeout, or a stop) is handled while a step hangs:
# bash runs a trap only between foreground commands, and `wait` is one it can interrupt.
unit_main &
UNIT_PID=$!
if wait "$UNIT_PID"; then UNIT_DONE=1; fi
kill "$UNIT_WATCHDOG" 2>/dev/null || true
