#!/usr/bin/env bash
# Shared helpers for the relay comparison harness. Sourced, not run.
#
# Every host is reached the same way: a command runs as root on a named
# role (control, relay, peer-a, peer-b). HOST_MODE=ssh reaches cloud
# machines over SSH; HOST_MODE=docker reaches the local lab's containers
# (local-lab.sh) with docker exec. Nothing else in the harness differs
# between the two.

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC2034  # used by the scripts that source this file
REMOTE_DIR="$HARNESS_DIR/remote"

die() { echo "error: $*" >&2; exit 1; }
log() { printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >&2; }

load_hosts() {
  local f=${HOSTS_FILE:-$HARNESS_DIR/hosts.env}
  [ -f "$f" ] || die "no hosts file at $f (copy hosts.env.example and fill it in)"
  # shellcheck source=/dev/null
  . "$f"
  : "${HOST_MODE:?HOST_MODE must be ssh or docker}"
  : "${RELAY_ADDR:?RELAY_ADDR (address the peers use for the relay) is required}"
  : "${CONTROL_ADDR:?CONTROL_ADDR (address the peers use for management) is required}"
  RELAY_PORT=${RELAY_PORT:-33080}
  MGMT_PORT=${MGMT_PORT:-8080}
}

# role_target <role> prints the SSH destination or container name.
role_target() {
  case "$1" in
    control) echo "$CONTROL_TARGET" ;;
    relay) echo "$RELAY_TARGET" ;;
    peer-a) echo "$PEER_A_TARGET" ;;
    peer-b) echo "$PEER_B_TARGET" ;;
    *) die "unknown role $1" ;;
  esac
}

ssh_opts() {
  printf '%s\n' -o BatchMode=yes -o StrictHostKeyChecking=accept-new \
    -o UserKnownHostsFile="${KNOWN_HOSTS:-$STATE_DIR/known_hosts}" \
    -o ConnectTimeout=15 -o ServerAliveInterval=15 -i "${SSH_KEY:-$HOME/.ssh/id_ed25519}"
}

# on <role> <command string>: run a shell command as root on a role.
on() {
  local role=$1; shift
  local t; t=$(role_target "$role")
  if [ "$HOST_MODE" = docker ]; then
    docker exec -i "$t" bash -c "$*"
  else
    # shellcheck disable=SC2046
    ssh $(ssh_opts) "$t" sudo bash -c "$(printf '%q' "$*")"
  fi
}

# on_script <role> <local script> [args...]: run a local script as root on a role.
on_script() {
  local role=$1 script=$2; shift 2
  local t; t=$(role_target "$role")
  local q=""
  local a
  for a in "$@"; do q="$q $(printf '%q' "$a")"; done
  if [ "$HOST_MODE" = docker ]; then
    docker exec -i "$t" bash -c "bash -s --$q" < "$script"
  else
    # shellcheck disable=SC2046,SC2029  # arguments are quoted with %q above
    ssh $(ssh_opts) "$t" "sudo bash -s --$q" < "$script"
  fi
}

# push <role> <local file> <remote path>: copy a file to a role (as root).
push() {
  local role=$1 src=$2 dst=$3
  local t; t=$(role_target "$role")
  if [ "$HOST_MODE" = docker ]; then
    docker exec "$t" mkdir -p "$(dirname "$dst")"
    docker cp "$src" "$t:$dst"
  else
    # shellcheck disable=SC2046,SC2029  # the path is quoted with %q
    ssh $(ssh_opts) "$t" "sudo mkdir -p $(printf '%q' "$(dirname "$dst")") && sudo tee $(printf '%q' "$dst") >/dev/null" < "$src"
  fi
}

# pull <role> <remote path> <local file>
pull() {
  local role=$1 src=$2 dst=$3
  on "$role" "cat $(printf '%q' "$src")" > "$dst"
}

# Versions under test. Override in hosts.env or the environment.
NETBIRD_VERSION=${NETBIRD_VERSION:-0.83.1}
GOAT_RELAY_REF=${GOAT_RELAY_REF:-goat-relay/v0.1.0}

STATE_DIR=${STATE_DIR:-$HARNESS_DIR/state}
BIN_DIR=${BIN_DIR:-$STATE_DIR/bin}
