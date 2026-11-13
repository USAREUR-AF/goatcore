#!/usr/bin/env bats
# Force-recreate env-staleness tests for ops/scripts/rollback.sh (#971).
#
# Every host action in rollback.sh goes over `ssh user@host "<remote script>"`.
# The fake `ssh` below runs that remote script LOCALLY (bash -c) against a
# throwaway fixture tree instead of a managed host, so no network, SSH key or
# real docker/ansible is touched. `sudo` is faked to exec through, rewriting
# the hard-coded /var/lib/goat/* runtime-ledger path to a writable fixture
# path. `docker` is faked to report a known running image and to no-op the
# compose recreate. `ansible-inventory` and `ops/scripts/ansible-preflight.sh`
# are faked as tripwires: invoking the full deploy would replace the sentinel
# environment and fail this test — rollback.sh calls the latter by
# its REPO_ROOT-relative absolute path, so the fixture tree is made to look
# like its own repo root (ops/scripts/rollback.sh copied in at that path) so
# that path resolves inside the fixture, never the real checkout.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
SUBJECT="$REPO_ROOT/ops/scripts/rollback.sh"

PREV_IMAGE="ghcr.io/example/microscope:sha-new@sha256:1111111111111111111111111111111111111111111111111111111111111a"
TARGET_IMAGE="ghcr.io/example/microscope:sha-old@sha256:2222222222222222222222222222222222222222222222222222222222222b"
HOST_IP="198.18.0.50"

setup() {
  command -v yq >/dev/null || skip "yq not installed"
  command -v jq >/dev/null || skip "jq not installed"

  FIXTURE="$BATS_TEST_TMPDIR/repo"
  BIN="$BATS_TEST_TMPDIR/bin"
  FAKE_VAR_LIB="$BATS_TEST_TMPDIR/var-lib-goat"
  COMPOSE_DIR="$FIXTURE/opt/microscope"
  mkdir -p "$FIXTURE/ops/scripts" "$FIXTURE/ops/ansible/roles/microscope" \
    "$FIXTURE/ops/ansible/inventory" "$COMPOSE_DIR" "$BIN" "$FAKE_VAR_LIB"

  cp "$SUBJECT" "$FIXTURE/ops/scripts/rollback.sh"
  chmod +x "$FIXTURE/ops/scripts/rollback.sh"

  # This fixture tests rollback's host-side image/environment behavior.
  # Source freshness has its own suite; no fixture commit or signing is needed.
  printf '#!/bin/sh\nexit 0\n' > "$FIXTURE/ops/scripts/fresh-source.sh"
  chmod +x "$FIXTURE/ops/scripts/fresh-source.sh"
  GOAT_FRESH_SOURCE_TRUNK="unused-fixture"

  # The ledger: one host, in-repo history only (no on-host jsonl ledger, so
  # resolve_artifact_meta and TARGET_IMAGE resolution fall back to it).
  cat > "$FIXTURE/ops/ansible/roles/microscope/deploy-history.yml" <<YAML
service: microscope
hosts:
  - host: ${HOST_IP}
    ssh_user: testuser
    container_name: microscope
    compose_path: ${COMPOSE_DIR}/docker-compose.yml
history:
  - sha: sha-new
    image: ${PREV_IMAGE}
    pr: 1
    merged_at: "2026-10-01T00:00:00Z"
  - sha: sha-old
    image: ${TARGET_IMAGE}
    pr: 2
    merged_at: "2026-09-01T00:00:00Z"
YAML

  cat > "$COMPOSE_DIR/docker-compose.yml" <<YAML
services:
  microscope:
    image: "${PREV_IMAGE}"
    env_file:
      - .env
YAML

  # The stale .env sitting on disk from the host's last real deploy — the
  # sentinel this test tracks across the force-recreate.
  printf 'MICROSCOPE_AUDIT_SECRET_KEY=STALE_SENTINEL_VALUE\n' > "$COMPOSE_DIR/.env"

  # The in-repo deploy playbook only needs to EXIST — ansible-preflight.sh is
  # faked below, so its content is never read.
  printf '# fixture, unused: ansible-preflight.sh is faked\n' > "$FIXTURE/ops/ansible/microscope-deploy.yml"

  # --- fakes on PATH --------------------------------------------------------
  # rollback.sh's embedded remote scripts assume GNU sed (the real target
  # hosts are Ubuntu), not the BSD sed this Mac's `sed` resolves to — fake
  # ssh puts GNU sed first on PATH for the command it runs, so the "remote"
  # script behaves like the real target instead of failing on a BSD-vs-GNU
  # difference unrelated to anything under test here.
  GNU_SED_DIR=""
  if [ -x /opt/homebrew/opt/gnu-sed/libexec/gnubin/sed ]; then
    GNU_SED_DIR=/opt/homebrew/opt/gnu-sed/libexec/gnubin
  elif sed --version 2>/dev/null | grep -q GNU; then
    GNU_SED_DIR="$(dirname "$(command -v sed)")"
  elif command -v gsed >/dev/null 2>&1; then
    GNU_SED_DIR="$(dirname "$(command -v gsed)")"
  fi
  [ -n "$GNU_SED_DIR" ] || skip "GNU sed (gsed) not installed — 'brew install gnu-sed' to run this suite; the real target hosts are Ubuntu/GNU sed, not this Mac's BSD sed"

  cat > "$BIN/ssh" <<SH
#!/usr/bin/env bash
# rollback.sh always calls \`ssh "\${SSH_OPTS[@]}" user@host "<remote script>"\`
# — the remote script is always the last argument. Run it locally.
cmd="\${*: -1}"
PATH="$BIN:$GNU_SED_DIR:\$PATH" bash -c "\$cmd"
SH
  chmod +x "$BIN/ssh"

  cat > "$BIN/sudo" <<SH
#!/usr/bin/env bash
args=()
for a in "\$@"; do
  args+=("\${a//\\/var\\/lib\\/goat/$FAKE_VAR_LIB}")
done
exec "\${args[@]}"
SH
  chmod +x "$BIN/sudo"

  cat > "$BIN/docker" <<SH
#!/usr/bin/env bash
case "\$1" in
  inspect) echo "$PREV_IMAGE" ;;
  compose) exit 0 ;;
  *) exit 0 ;;
esac
SH
  chmod +x "$BIN/docker"

  cat > "$BIN/ansible-inventory" <<SH
#!/usr/bin/env bash
cat <<JSON
{"_meta":{"hostvars":{"fake-microscope-host":{"ansible_host":"$HOST_IP"}}}}
JSON
SH
  chmod +x "$BIN/ansible-inventory"

  # ansible-preflight.sh is called by rollback.sh at its REPO_ROOT-relative
  # absolute path, so it is faked IN PLACE in the fixture tree, not on PATH.
  cat > "$FIXTURE/ops/scripts/ansible-preflight.sh" <<'SH'
#!/usr/bin/env bash
# Simulates a successful ansible render: writes the fresh .env a real apply
# of the role's `render .env` task would produce, replacing the stale copy.
printf 'MICROSCOPE_AUDIT_SECRET_KEY=fresh-value-from-render\n' > "$FAKE_ENV_PATH"
exit 0
SH
  chmod +x "$FIXTURE/ops/scripts/ansible-preflight.sh"
}

run_rollback() {
  cd "$FIXTURE" || return
  run env PATH="$BIN:$PATH" GOAT_FRESH_SOURCE_TRUNK="$GOAT_FRESH_SOURCE_TRUNK" \
    FAKE_VAR_LIB="$FAKE_VAR_LIB" FAKE_ENV_PATH="$COMPOSE_DIR/.env" \
    "${BASH_UNDER_TEST:-/bin/bash}" ./ops/scripts/rollback.sh "$@"
}

@test "rollback warns about the reused environment and never runs a full deployment" {
  run_rollback microscope 1

  [ "$status" -eq 0 ] || { printf "%s\n" "$output" >&3; return 1; }
  env_contents="$(cat "$COMPOSE_DIR/.env")"
  [[ "$env_contents" == *"STALE_SENTINEL_VALUE"* ]]
  [[ "$env_contents" != *"fresh-value-from-render"* ]]
  [[ "$output" == *"rollback does not refresh it"* ]] || { printf "%s\n" "$output" >&3; return 1; }
  [[ "$output" == *"compatible with"* ]]
  [[ "$(cat "$COMPOSE_DIR/docker-compose.yml")" == *"$TARGET_IMAGE"* ]]
}

# Emit a matching first record before the rest, making an early-closing reader
# observable. Only the history membership query is intercepted; all other yq
# calls and all host actions use the normal isolated fixture above.
install_history_probe() {
  command -v python3 >/dev/null || skip "python3 not installed"
  export REAL_HISTORY_YQ
  REAL_HISTORY_YQ="$(command -v yq)"
  cat > "$BIN/yq" <<'PYTHON'
#!/usr/bin/env python3
import os
import subprocess
import sys
import time

if ".history[].image" not in sys.argv:
    os.execv(os.environ["REAL_HISTORY_YQ"], ["yq", *sys.argv[1:]])
result = subprocess.run([os.environ["REAL_HISTORY_YQ"], *sys.argv[1:]], capture_output=True)
if result.returncode:
    sys.stderr.buffer.write(result.stderr)
    sys.exit(result.returncode)
mode = os.environ["HISTORY_PROBE_MODE"]
lines = result.stdout.splitlines(keepends=True)
if mode == "unknown":
    lines = [b"ghcr.io/example/unrelated:sha-other\n"]
for line in lines:
    sys.stdout.buffer.write(line)
    sys.stdout.buffer.flush()
    time.sleep(0.1)
sys.exit(7 if mode == "failed-reader" else 0)
PYTHON
  chmod +x "$BIN/yq"
}

@test "rollback drains history after a known first record" {
  install_history_probe
  export HISTORY_PROBE_MODE=slow
  run_rollback microscope 1
  [ "$status" -eq 0 ] || { printf "%s\n" "$output" >&3; return 1; }
  [[ "$(cat "$COMPOSE_DIR/docker-compose.yml")" == *"$TARGET_IMAGE"* ]]
  [[ "$(cat "$COMPOSE_DIR/.env")" == *"STALE_SENTINEL_VALUE"* ]]
}

@test "rollback refuses an unknown image without changing compose" {
  install_history_probe
  export HISTORY_PROBE_MODE=unknown
  run_rollback microscope 1
  [ "$status" -ne 0 ]
  [[ "$output" == *"NO known history"* ]]
  [[ "$(cat "$COMPOSE_DIR/docker-compose.yml")" == *"$PREV_IMAGE"* ]]
}

@test "rollback refuses a failed history reader even after a matching record" {
  install_history_probe
  export HISTORY_PROBE_MODE=failed-reader
  run_rollback microscope 1
  [ "$status" -ne 0 ]
  [[ "$output" == *"NO known history"* ]]
  [[ "$(cat "$COMPOSE_DIR/docker-compose.yml")" == *"$PREV_IMAGE"* ]]
}

@test "declared restore-only policy refuses before unavailable or legacy host ledgers" {
  mkdir -p "$FIXTURE/ops/ansible/roles/microscope/meta"
  cat > "$FIXTURE/ops/ansible/roles/microscope/meta/main.yml" <<'YAML'
dependencies:
  - role: rollback-ledger
    vars:
      rollback_reversible: false
      rollback_restore_pointer: consistent-state snapshot required
YAML
  # Any host read would cross this tripwire. Source policy must decide first.
  cat > "$BIN/ssh" <<'SH'
#!/bin/sh
echo unexpected-host-read >&2
exit 99
SH
  chmod +x "$BIN/ssh"
  run_rollback microscope 1
  [ "$status" -eq 3 ] || { printf '%s\n' "$output" >&3; return 1; }
  [[ "$output" == *"consistent-state snapshot required"* ]]
  [[ "$output" != *"unexpected-host-read"* ]]
  [[ "$(cat "$COMPOSE_DIR/docker-compose.yml")" == *"$PREV_IMAGE"* ]]
  [[ "$(cat "$COMPOSE_DIR/.env")" == *"STALE_SENTINEL_VALUE"* ]]
}
