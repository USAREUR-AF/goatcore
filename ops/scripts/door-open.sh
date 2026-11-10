#!/usr/bin/env bash
# door-open.sh — open an env door and prove it landed, in one command.
#
# ops/scripts/env-door.sh runs the SSM port-forward in the FOREGROUND, so using
# it means backgrounding it by hand, guessing how long to wait, and finding out
# it never came up when an ansible run reports `unreachable=1` against a host
# that is perfectly healthy. This wrapper does the waiting and the proving.
#
# Door facts (port, user, target host) are NOT restated here — they are read
# back out of env-door.sh, which is the one place they are decided (ADR 1117,
# docs/design/env-doors.md).
#
# usage:
#   ops/scripts/door-open.sh <env>           # open (or reuse) the door, verify, print usage
#   ops/scripts/door-open.sh <env> --status  # report only; never opens anything
#   ops/scripts/door-open.sh <env> --close   # close a door this script opened
#
# envs: whatever env-door.sh accepts .
set -uo pipefail

ENV="${1:-}"
MODE="${2:---open}"
[ -n "$ENV" ] || { echo "usage: $0 <env> [--status|--close]" >&2; exit 1; }

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_DOOR="${REPO_ROOT}/scripts/env-door.sh"
# The AWS profile the printed deploy line runs under: GOAT_AWS_PROFILE, else
# the checkout's deploy.aws_profile, else `default`, the order env-door.sh reads.
AWS_PROFILE_NAME="${GOAT_AWS_PROFILE:-$("${REPO_ROOT}/scripts/project-setting.sh" deploy aws_profile 2>/dev/null)}"
AWS_PROFILE_NAME="${AWS_PROFILE_NAME:-default}"
RUN_DIR="${TMPDIR:-/tmp}/goat-doors"
mkdir -p "$RUN_DIR"

# env-door.sh calls `aws-vault exec` itself. Wrapping it fails with "running in
# an existing aws-vault subshell" — and because a door may already be listening
# from an earlier run, the port check then looks like success while this
# invocation did nothing at all. Refuse instead of half-working.
if [ -n "${AWS_VAULT:-}" ]; then
  echo "door-open: refusing to run inside an aws-vault subshell (AWS_VAULT=${AWS_VAULT})." >&2
  echo "           env-door.sh calls aws-vault itself. Run this script bare." >&2
  exit 1
fi

# Read the door's facts from the script that owns them. --print also runs
# env-door.sh's own assert that the two door inventories agree with it, so a
# drifted door fails here rather than connecting somewhere unintended.
FACTS=$("$ENV_DOOR" "$ENV" --print 2>&1) || { echo "$FACTS" >&2; exit 1; }
PORT=$(echo "$FACTS" | sed -n 's/.*local port \([0-9]*\).*/\1/p' | head -1)
TARGET=$(echo "$FACTS" | sed -n 's/^# door: [^ ]* -> \([^ ]*\) .*/\1/p' | head -1)
USER_=$(echo "$FACTS" | sed -n "s/^#   ssh -p ${PORT} \([^@]*\)@127.0.0.1.*/\1/p" | head -1)
[ -n "$PORT" ] && [ -n "$TARGET" ] && [ -n "$USER_" ] || {
  echo "door-open: could not read port/target/user out of env-door.sh --print." >&2
  echo "           That script's output shape changed; fix the parse here." >&2
  exit 1
}
PIDFILE="${RUN_DIR}/${ENV}.pid"
LOGFILE="${RUN_DIR}/${ENV}.log"

port_open() { nc -z 127.0.0.1 "$PORT" >/dev/null 2>&1; }

# An open local port only proves something is listening. SSM keeps the local
# listener up while the far end is gone, and an SSM session idle-times out at
# ~20 min, so "port open" and "door usable" are different facts. Only an ssh
# that reaches the host settles it.
ssh_lands() {
  ssh -p "$PORT" -o BatchMode=yes -o ConnectTimeout=10 \
      -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
      "${USER_}@127.0.0.1" true >/dev/null 2>&1
}

case "$MODE" in
  --status)
    if port_open && ssh_lands; then echo "door ${ENV}: OPEN and landing on ${TARGET} (127.0.0.1:${PORT})"; exit 0
    elif port_open; then echo "door ${ENV}: port ${PORT} listening but ssh does not land — stale session, re-open"; exit 1
    else echo "door ${ENV}: closed"; exit 1; fi
    ;;
  --close)
    if [ -f "$PIDFILE" ]; then
      kill "$(cat "$PIDFILE")" 2>/dev/null && echo "door ${ENV}: closed (pid $(cat "$PIDFILE"))"
      rm -f "$PIDFILE"
    else
      echo "door ${ENV}: no pidfile here; if a door is open it was started elsewhere." >&2
      exit 1
    fi
    exit 0
    ;;
esac

if port_open && ssh_lands; then
  echo "door ${ENV}: already open, ssh lands on ${TARGET}"
else
  # A listening port whose ssh does not land is a timed-out session holding the
  # port. Kill ours before re-opening, or the new session cannot bind.
  if port_open && [ -f "$PIDFILE" ]; then
    echo "door ${ENV}: stale session on ${PORT}, replacing it"
    kill "$(cat "$PIDFILE")" 2>/dev/null; rm -f "$PIDFILE"; sleep 2
  fi
  echo "door ${ENV}: opening via env-door.sh (log: ${LOGFILE})"
  nohup "$ENV_DOOR" "$ENV" >"$LOGFILE" 2>&1 &
  echo $! >"$PIDFILE"

  for _ in $(seq 1 30); do port_open && break; sleep 1; done
  if ! port_open; then
    echo "door ${ENV}: port ${PORT} never came up after 30s. Last output:" >&2
    tail -5 "$LOGFILE" >&2
    exit 1
  fi
  for _ in $(seq 1 15); do ssh_lands && break; sleep 1; done
  if ! ssh_lands; then
    echo "door ${ENV}: port ${PORT} is listening but ssh does not land on ${TARGET}." >&2
    echo "            The SSM hop worked and the mesh hop did not — check that the door" >&2
    echo "            peer is still a connected peer of the ${ENV} mesh before blaming the host." >&2
    tail -5 "$LOGFILE" >&2
    exit 1
  fi
  echo "door ${ENV}: OPEN, ssh lands on ${TARGET}"
fi

TARGET_GROUP="${ENV//-/_}_hosts"
cat <<EOF

  interactive:
    ssh -p ${PORT} -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null ${USER_}@127.0.0.1

  another peer in ${ENV} (jumped through ${TARGET}):
    ssh -o ProxyJump=${USER_}@127.0.0.1:${PORT} ${USER_}@<peer-mesh-ip>

  host-level play (no site group_vars):
    cd ops/ansible && ansible-playbook -i inventory/doors.ini <play>.yml

  service deploy (templates from group_vars — base inventory FIRST):
    cd ops/ansible && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES \\
      aws-vault exec ${AWS_PROFILE_NAME} --no-session -- ../scripts/ansible-preflight.sh \\
        -i inventory/mgmt-hosts.ini -i inventory/doors-groupvars.ini \\
        <play>.yml -e target=${TARGET_GROUP} 2>&1 | tee /tmp/deploy-${ENV}.log

  Doors idle out at ~20 min. Re-run this script rather than debugging a host:
    ops/scripts/door-open.sh ${ENV} --status
EOF
