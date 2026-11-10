#!/usr/bin/env bash
# env-door.sh — open a mesh-independent door into one goatnet's mgmt host.
#
# Design: docs/design/env-doors.md (ADR 1117). Tier 1: SSM port-forward via
# the env's cloud mesh peer; SSH authenticates end-to-end with YOUR key, the
# cloud peer only moves ciphertext. The laptop's netbird profile is not
# touched — run one door per env, in parallel, from any session.
#
# usage:
#   ops/scripts/env-door.sh <env>            # open the tunnel (foreground; Ctrl-C closes)
#   ops/scripts/env-door.sh <env> --print    # print ssh/ansible usage, don't connect
#   ops/scripts/env-door.sh <env> --ssh-target   # user@127.0.0.1:port of the door
#   ops/scripts/env-door.sh <env> --host-target  # user@<mesh address> of the host behind it
#
# envs: every goatnet with an ssm door in the door registry
# (ops/ansible/inventory/doors.yaml); a joiner site rides its founder's door.
set -euo pipefail

ENV="${1:-}"
MODE="${2:-run}"
# An unrecognised flag must not fall through to opening a tunnel in the
# foreground: a caller asking an older copy of this script for a mode it lacks
# would hang on a tunnel instead of failing.
case "$MODE" in
  run|--print|--ssh-target|--host-target) ;;
  *) echo "usage: $0 <goatnet> [--print|--ssh-target|--host-target]" >&2
     echo "env-door: unknown mode '$MODE'" >&2; exit 1 ;;
esac
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# The door's facts come from the registry, read at run time: the script names no
# goatnet. In an assembled tree (goatcore + an environment overlay) this script
# is goatcore's and the registry is the overlay's. Plain KEY=token lines only;
# never sourced.
# The registry is YAML: python3 when it has PyYAML, else Ansible's own
# interpreter, which always does (the choice deploy-component.sh makes).
PY="${GOATBUILDER_PYTHON:-python3}"
if ! "$PY" -c 'import yaml' 2>/dev/null; then
  _ap="$(head -1 "$(command -v ansible-playbook 2>/dev/null || echo /dev/null)" 2>/dev/null | sed 's/^#!//' | awk '{print $NF}' || true)"
  if [ -n "$_ap" ] && "$_ap" -c 'import yaml' 2>/dev/null; then PY="$_ap"; fi
fi
_facts=""
[ -z "$ENV" ] || _facts="$("$PY" "$HERE/generate-door-inventory.py" --arm "$ENV" 2>/dev/null || true)"
if [ -z "$_facts" ]; then
  echo "usage: $0 <goatnet> [--print|--ssh-target|--host-target]" >&2
  [ -z "$ENV" ] || echo "env-door: no ssm door for '$ENV' in ops/ansible/inventory/doors.yaml (or the registry could not be read: it needs python3 with PyYAML)." >&2
  exit 1
fi
while IFS='=' read -r _k _v; do
  case "$_k" in
    INSTANCE) INSTANCE="$_v" ;; REGION) REGION="$_v" ;; TARGET_IP) TARGET_IP="$_v" ;;
    TARGET_NAME) TARGET_NAME="$_v" ;; PORT) PORT="$_v" ;; DOOR_USER) DOOR_USER="$_v" ;;
  esac
done <<< "$_facts"
unset _facts _k _v

# --ssh-target: the one line a caller needs to run something on the host through
# an OPEN door (door-open.sh <env> first): user@127.0.0.1:port. The release
# driver's rows use it (goatnet-runners.yaml `goat_via: door:<env>`).
if [ "$MODE" = "--ssh-target" ]; then printf '%s@127.0.0.1:%s\n' "$DOOR_USER" "$PORT"; exit 0; fi
# --host-target: the management host itself, user@<mesh address>, for an ssh
# that jumps through the open door (ProxyCommand to --ssh-target, then -W to
# this). The door port is a tunnel whose host key churns; the host's own key
# is the one a caller verifies (ops/validation/probes.py _ssh_jump).
if [ "$MODE" = "--host-target" ]; then printf '%s@%s\n' "$DOOR_USER" "$TARGET_IP"; exit 0; fi

cat <<EOF
# door: $ENV -> $TARGET_NAME ($TARGET_IP) via $INSTANCE, local port $PORT
#
# interactive:
#   ssh -p $PORT $DOOR_USER@127.0.0.1
# fleet (any $ENV mesh peer, jumped through the mgmt host):
#   ssh -o ProxyJump=$DOOR_USER@127.0.0.1:$PORT $DOOR_USER@<peer-mesh-ip>
# ansible, host-level play (no site group_vars — wireguard-kmod.yml et al):
#   ansible-playbook -i inventory/doors.ini <play>.yml
# ansible, SERVICE deploy that templates from group_vars:
#   ansible-playbook -i inventory/mgmt-hosts.ini -i inventory/doors-groupvars.ini \\
#     <play>.yml -e target=${ENV//-/_}_hosts
#   (base inventory first for group membership + router_role; the door file
#    overrides transport only. Do NOT also pass mgmt-hosts-mesh.ini.)
# host-key churn note: pair with
#   -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
EOF

# One-fact-one-home assert: this script decides the door's port and user, and
# two inventories restate them. A door whose inventories disagree does not fail
# loudly — it produces "unreachable", or worse, connects somewhere else. Refuse
# to open rather than let the operator debug a transport that was never going to
# work. Both files are checked because a fix applied to one is the likely drift.
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
door_drift=0
for inv in "${REPO_ROOT}/ansible/inventory/doors.ini" \
           "${REPO_ROOT}/ansible/inventory/doors-groupvars.ini"; do
  [ -f "$inv" ] || { echo "env-door: missing inventory $inv" >&2; door_drift=1; continue; }
  if ! grep -q "^${TARGET_NAME} .*ansible_port=${PORT} .*ansible_user=${DOOR_USER}" "$inv"; then
    echo "env-door: $(basename "$inv") does not carry ${TARGET_NAME} on port ${PORT} as ${DOOR_USER}." >&2
    door_drift=1
  fi
done
if [ "$door_drift" != 0 ]; then
  echo "env-door: REFUSING to open — the door facts here and in the inventories disagree." >&2
  echo "          This script is where the port/user are decided; fix the inventory to match." >&2
  exit 1
fi

[ "$MODE" = "--print" ] && exit 0

command -v session-manager-plugin >/dev/null || {
  echo "session-manager-plugin not found (brew install --cask session-manager-plugin)" >&2
  exit 1
}

# The AWS profile the door opens under is the project's: GOAT_AWS_PROFILE for
# one command, then deploy.aws_profile in ops/goatbuilder/goatbuilder.yaml, then
# `default`, the name the AWS tools fall back to themselves.
AWS_PROFILE_DOOR="${GOAT_AWS_PROFILE:-$("$HERE/project-setting.sh" deploy aws_profile 2>/dev/null || true)}"
[ -n "$AWS_PROFILE_DOOR" ] || AWS_PROFILE_DOOR=default

exec "$HERE/with-aws" "$AWS_PROFILE_DOOR" -- \
  aws ssm start-session \
    --target "$INSTANCE" \
    --region "$REGION" \
    --document-name AWS-StartPortForwardingSessionToRemoteHost \
    --parameters "{\"host\":[\"$TARGET_IP\"],\"portNumber\":[\"22\"],\"localPortNumber\":[\"$PORT\"]}"
