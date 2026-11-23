#!/usr/bin/env bats

# Tests for ops/ansible/roles/mgmt-host-compose-watch/files/mgmt-host-compose-watch.sh
# (#1364 restart-policy drift, #1365 all-projects reporting). Stubs docker,
# docker compose and jq so the watcher's metric-emit path runs against a
# scripted fleet without a real Docker daemon. The watcher's main loop never
# terminates on its own (`while :; do ... done`); the stubbed `sleep`
# signals the loop's own SIGTERM trap after the first cycle so a single
# emit_metrics() pass completes and the process exits 0.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
SUBJECT="$REPO_ROOT/ops/ansible/roles/mgmt-host-compose-watch/files/mgmt-host-compose-watch.sh"

setup() {
  if [ ! -f "$SUBJECT" ]; then
    echo "subject script not found at $SUBJECT" >&2
    return 1
  fi
  for tool in jq timeout; do
    command -v "$tool" >/dev/null 2>&1 || skip "$tool not available"
  done

  fixture="$BATS_TEST_TMPDIR/fixture"
  compose_dir="$fixture/compose-dir"
  metrics_dir="$fixture/metrics"
  bin="$fixture/bin"
  mkdir -p "$compose_dir" "$metrics_dir" "$bin"

  # Minimum needed for discover_compose_files() to populate COMPOSE_FILES.
  : > "$compose_dir/docker-compose.yml"

  # Stub `docker` — dispatches on the full argv so the test does not care
  # about flag ordering, only about which query the script is making.
  #
  # Fixture fleet, three projects:
  #   mgmt-host / dashboard       running  — declared unless-stopped, live no    -> DRIFT
  #   mgmt-host / goat-get        running  — declared unless-stopped, live same  -> no drift
  #   mgmt-host / kratos-migrate  exited   — ONESHOT, must be skipped everywhere
  #   nova-obs    / nova-obs-grafana  exited   — a DIFFERENT project; must now be
  #                                          reported (that is #1365) but
  #                                          never fed into the mgmt-host-only
  #                                          drift gauge (#1364's scope decision)
  #   zenoh-rig / rig-L8          running  — EPHEMERAL, must be skipped
  cat > "$bin/docker" <<'DOCKER_STUB'
#!/usr/bin/env bash
args="$*"
case "$args" in
  *"ps -a --filter label=com.docker.compose.project=mgmt-host --format"*)
    cat <<'EOF'
dashboard running cid-dashboard
goat-get running cid-goatget
kratos-migrate exited cid-migrate
EOF
    ;;
  *"ps -a --filter label=com.docker.compose.project --format"*)
    cat <<'EOF'
mgmt-host dashboard running
mgmt-host goat-get running
mgmt-host kratos-migrate exited
nova-obs nova-obs-grafana exited
zenoh-rig rig-L8 running
EOF
    ;;
  *"compose "*"config --format json"*)
    printf '%s\n' '{"services":{"dashboard":{"restart":"unless-stopped"},"goat-get":{"restart":"unless-stopped"},"kratos-migrate":{"restart":"no"}}}'
    ;;
  *"compose "*"ps --status=exited --format"*)
    # TEST_EXITED: the services the stand-in reports as exited (none by default)
    [ -z "${TEST_EXITED:-}" ] || printf '%s\n' $TEST_EXITED
    ;;
  *"compose "*" up -d "*)
    printf 'up %s\n' "${args##* }" >> "$TEST_REVIVE_LOG"
    ;;
  *"inspect --format {{.HostConfig.RestartPolicy.Name}} cid-dashboard"*)
    [ "${TEST_INSPECT_FAIL:-0}" != 1 ] || exit 1
    echo "no"
    ;;
  *"inspect --format {{.HostConfig.RestartPolicy.Name}} cid-goatget"*)
    echo "unless-stopped"
    ;;
  *)
    exit 0
    ;;
esac
DOCKER_STUB
  chmod +x "$bin/docker"

  # Stub `sleep` — the real watcher calls it once per cycle at the bottom of
  # the loop. Signal the watcher's own PID (its parent, since this stub runs
  # as a child of it) so the script's existing `trap ... TERM INT` fires and
  # exits 0 after exactly one cycle, instead of looping forever.
  cat > "$bin/sleep" <<'SLEEP_STUB'
#!/usr/bin/env bash
kill -TERM "$PPID" 2>/dev/null
exit 0
SLEEP_STUB
  chmod +x "$bin/sleep"

  export PATH="$bin:$PATH"
  export MGMT_HOST_COMPOSE_DIR="$compose_dir"
  export MGMT_HOST_COMPOSE_WATCH_INTERVAL=1
  export MGMT_HOST_COMPOSE_WATCH_METRICS_DIR="$metrics_dir"
  export MGMT_HOST_COMPOSE_WATCH_SITE="testsite"
  export MGMT_HOST_COMPOSE_WATCH_SERVICES=""
  export MGMT_HOST_COMPOSE_WATCH_PAUSE_FILE="$fixture/compose-watch.paused"
  export TEST_REVIVE_LOG="$fixture/revived"
  : > "$TEST_REVIVE_LOG"
  metrics_file="$metrics_dir/compose_service_running.prom"
}

@test "restart-policy drift gauge fires for a container whose live RestartPolicy disagrees with the rendered compose file" {
  run timeout 10 bash "$SUBJECT"
  [ "$status" -eq 0 ]
  [ -f "$metrics_file" ]
  grep -q '^goat_mgmt_host_restart_policy_drift{service="dashboard",site="testsite",declared="unless-stopped",live="no"} 1$' "$metrics_file"
}

@test "restart-policy drift gauge reports 0 for a container that matches its compose file" {
  run timeout 10 bash "$SUBJECT"
  [ "$status" -eq 0 ]
  grep -q '^goat_mgmt_host_restart_policy_drift{service="goat-get",site="testsite",declared="unless-stopped",live="unless-stopped"} 0$' "$metrics_file"
}

@test "restart-policy drift gauge skips a one-shot migration container" {
  run timeout 10 bash "$SUBJECT"
  [ "$status" -eq 0 ]
  ! grep -q 'goat_mgmt_host_restart_policy_drift{service="kratos-migrate"' "$metrics_file"
}

@test "compose_service_running now covers a project other than mgmt-host (#1365)" {
  run timeout 10 bash "$SUBJECT"
  [ "$status" -eq 0 ]
  grep -q '^compose_service_running{service="nova-obs-grafana",site="testsite",state="exited",project="nova-obs"} 0$' "$metrics_file"
}

@test "compose_service_running still reports the mgmt-host project, now labelled" {
  run timeout 10 bash "$SUBJECT"
  [ "$status" -eq 0 ]
  grep -q '^compose_service_running{service="dashboard",site="testsite",state="running",project="mgmt-host"} 1$' "$metrics_file"
}

@test "compose_service_running exempts a one-shot migration container from the widened report" {
  run timeout 10 bash "$SUBJECT"
  [ "$status" -eq 0 ]
  ! grep -q 'compose_service_running{service="kratos-migrate"' "$metrics_file"
}

@test "compose_service_running exempts an ephemeral load-rig container from the widened report" {
  run timeout 10 bash "$SUBJECT"
  [ "$status" -eq 0 ]
  ! grep -q 'compose_service_running{service="rig-L8"' "$metrics_file"
}

@test "failed inspection is unmeasured rather than restart-policy no" {
  export TEST_INSPECT_FAIL=1
  run timeout 10 bash "$SUBJECT"
  [ "$status" -eq 0 ]
  ! grep -q 'goat_mgmt_host_restart_policy_drift{service="dashboard"' "$metrics_file"
  grep -q 'goat_mgmt_host_restart_policy_drift{service="goat-get"' "$metrics_file"
}

# The pause file: a procedure that stops a service on purpose (the joiner's
# Postgres re-initialisation) sets it first, so the watcher does not revive the
# service it stopped. The file lives under /run, so a reboot clears it.

@test "an exited service is revived when the watcher is not paused" {
  export TEST_EXITED="postgres-a"
  run timeout 10 bash "$SUBJECT"
  [ "$status" -eq 0 ]
  grep -qx 'up postgres-a' "$TEST_REVIVE_LOG"
  grep -q '^compose_watch_paused{site="testsite"} 0$' "$metrics_file"
}

@test "while the pause file exists, an exited service is not revived" {
  export TEST_EXITED="postgres-a"
  : > "$MGMT_HOST_COMPOSE_WATCH_PAUSE_FILE"
  run timeout 10 bash "$SUBJECT"
  [ "$status" -eq 0 ]
  [ ! -s "$TEST_REVIVE_LOG" ]
  [[ "$output" == *"paused"* ]]
  [[ "$output" == *"$MGMT_HOST_COMPOSE_WATCH_PAUSE_FILE"* ]]
  grep -q '^compose_watch_paused{site="testsite"} 1$' "$metrics_file"
}
