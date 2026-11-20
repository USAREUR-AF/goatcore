#!/usr/bin/env bash
# tasks/retire-runner.yml removes the retired sidecar from a host that has it:
# a RUNNING container, a stopped one, the overlay file and the .env block, and
# leaves the bind-mounted repository alone. A second run changes nothing.
# Needs ansible-playbook and docker; runs with no privilege. The container is
# named per run, so it never touches a real pgbackrest-runner.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
AP="${ANSIBLE_PLAYBOOK:-ansible-playbook}"
IMG="${PGBR_RETIRE_TEST_IMAGE:-alpine:3.24@sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf}"
command -v "$AP" >/dev/null 2>&1 || { echo "[retire-runner] SKIP (ansible-playbook not on PATH)"; exit 0; }
command -v docker >/dev/null 2>&1 || { echo "[retire-runner] SKIP (docker not on PATH)"; exit 0; }
export ANSIBLE_BECOME=False
W="$(mktemp -d)"
NAME="pgbr-retire-test-$$"
cleanup() { docker rm -fv "$NAME" >/dev/null 2>&1; rm -rf "$W"; }
trap cleanup EXIT
fails=0
ok() { echo "ok   $*"; }
bad() { echo "FAIL $*"; fails=$((fails + 1)); }
fixture() {
  mkdir -p "$W/compose" "$W/repo/backup/st"
  echo "services: {}" > "$W/compose/docker-compose.pgbackrest.yml"
  printf 'A=1\n# BEGIN ANSIBLE MANAGED: pgbackrest\nPGBACKREST_IMAGE=example\n# END ANSIBLE MANAGED: pgbackrest\nB=2\n' > "$W/compose/.env"
  echo x > "$W/repo/backup/st/backup.info"
}
run() {
  (cd "$HERE" && "$AP" -i 'localhost,' retire-runner.yml \
    -e "compose_dir=$W/compose" -e "pgbackrest_retired_runner_container=$NAME" 2>&1)
}
changed() { grep -oE 'changed=[0-9]+' <<<"$1" | tail -1 | cut -d= -f2; }
check_removed() {
  local label=$1 out=$2
  if docker container inspect "$NAME" >/dev/null 2>&1; then bad "$label: the container is still there"; else ok "$label: the container is removed"; fi
  if [ -e "$W/compose/docker-compose.pgbackrest.yml" ]; then bad "$label: the overlay is still there"; else ok "$label: the overlay is removed"; fi
  if grep -q PGBACKREST_IMAGE "$W/compose/.env"; then bad "$label: the .env block is still there"
  elif grep -qx A=1 "$W/compose/.env" && grep -qx B=2 "$W/compose/.env"; then ok "$label: the .env block is removed, the rest kept"
  else bad "$label: .env lost other lines"; fi
  if [ -f "$W/repo/backup/st/backup.info" ]; then ok "$label: the repository is untouched"; else bad "$label: the repository changed"; fi
  if [ "$(changed "$out")" -ge 1 ]; then ok "$label: reported changed"
  else bad "$label: not reported changed: $(grep -E 'ok=|failed=' <<<"$out" | tail -1)"; fi
}

# Case 1: a RUNNING sidecar, as on a host where compose started it.
fixture
docker run -d --name "$NAME" -v "$W/repo:/var/lib/pgbackrest" "$IMG" sleep 600 >/dev/null
[ "$(docker container inspect -f '{{.State.Running}}' "$NAME")" = true ] || bad "case 1: the fixture container is not running"
if out=$(run); then check_removed "running" "$out"; else bad "running: the play failed"; echo "$out" | tail -8; fi

# Case 2: the same again: nothing left to remove, nothing changed, no error.
if out=$(run); then
  if [ "$(changed "$out")" = 0 ]; then ok "rerun: nothing changed"; else bad "rerun: changed=$(changed "$out")"; fi
else bad "rerun: the play failed with nothing to remove"; echo "$out" | tail -8; fi

# Case 3: a STOPPED sidecar.
fixture
docker rm -fv "$NAME" >/dev/null 2>&1; docker create --name "$NAME" "$IMG" true >/dev/null
if out=$(run); then check_removed "stopped" "$out"; else bad "stopped: the play failed"; echo "$out" | tail -8; fi

if [ "$fails" -ne 0 ]; then echo "[retire-runner] FAIL ($fails)"; exit 1; fi
echo "[retire-runner] PASS"
