#!/usr/bin/env bats
# deploy-component.sh over a door: the door is re-opened before each group's
# play and before the ledgers are read, a door that fails to open is tried
# twice, and a play that lost its connection (UNREACHABLE) runs once more,
# while a play whose task failed does not. Every host and credential command is
# stubbed; nothing here contacts a goatnet.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"

setup() {
  command -v python3 >/dev/null || skip "python3 not installed"
  python3 -c 'import yaml' 2>/dev/null || skip "PyYAML not installed"
  FIXTURE="$BATS_TEST_TMPDIR/repo"
  STUBS="$BATS_TEST_TMPDIR/stubs"
  BIN="$BATS_TEST_TMPDIR/bin"
  mkdir -p "$FIXTURE/ops/scripts" "$FIXTURE/ops/pins" \
    "$FIXTURE/ops/ansible/group_vars/test_hosts" "$STUBS" "$BIN"
  mkdir -p "$FIXTURE/ops/release" && cp "$REPO_ROOT/ops/release/ladder_gate.py" "$FIXTURE/ops/release/"  # the release-ladder gate (GB-373); no ladder declared here, so it passes
  cp "$REPO_ROOT/ops/scripts/deploy-component.sh" "$REPO_ROOT/ops/scripts/served-payloads.py" \
    "$REPO_ROOT/ops/scripts/fresh-source.sh" "$FIXTURE/ops/scripts/"
  # The intent check (GB-383) is tested by crates/goatbuilder/tests/deploy_dry_run_refusals.rs.
  printf 'print("ok: signature valid")\n' > "$FIXTURE/ops/pins/generate-pins.py"
  cat > "$FIXTURE/ops/pins/fleet-pins.yaml" <<'YAML'
goatnets:
  test_site:
    members: [test_hosts]
components:
  widget:
    var: widget_image
    fleet: example/widget@sha256:7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57
    deploy_playbook: widget-deploy.yml
YAML
  echo 'widget_image: "example/widget@sha256:7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57"' > "$FIXTURE/ops/ansible/group_vars/test_hosts/pins.yml"
  echo 'hosts: "{{ target }}"' > "$FIXTURE/ops/ansible/widget-deploy.yml"
  # This goatnet is reached through a door.
  printf '#!/usr/bin/env bash\nexit 0\n' > "$FIXTURE/ops/scripts/env-door.sh"
  # door-open.sh: counts its calls; fails while $STUBS/door-fail holds a count > 0.
  cat > "$FIXTURE/ops/scripts/door-open.sh" <<'SH'
#!/usr/bin/env bash
echo x >> "$STUBS/door-calls"
n="$(cat "$STUBS/door-fail" 2>/dev/null || echo 0)"
if [ "$n" -gt 0 ]; then echo $((n - 1)) > "$STUBS/door-fail"; exit 1; fi
exit 0
SH
  printf '#!/usr/bin/env bash\nprintf "Profile: x\\nManagement: Disconnected\\n"\n' > "$BIN/netbird"
  # with-aws: the play's outcomes come from $STUBS/plays, one line per run:
  # ok | unreachable | failed.
  cat > "$FIXTURE/ops/scripts/with-aws" <<'SH'
#!/usr/bin/env bash
shift 2
case "$1" in
  ansible-inventory) cat "$STUBS/inventory"; exit 0 ;;
  ansible) cat "$STUBS/rows"; exit 0 ;;
  ../scripts/ansible-preflight.sh)
    echo x >> "$STUBS/play-calls"
    n="$(wc -l < "$STUBS/play-calls" | tr -d ' ')"
    outcome="$(sed -n "${n}p" "$STUBS/plays")"
    case "${outcome:-ok}" in
      ok) printf 'PLAY RECAP\nhost-a : ok=1 changed=1 unreachable=0 failed=0\n'; exit 0 ;;
      unreachable) printf 'fatal: [host-a]: UNREACHABLE! => {"msg": "Connection reset by peer"}\nPLAY RECAP\nhost-a : ok=6 changed=1 unreachable=1 failed=0\n'; exit 4 ;;
      failed) printf 'fatal: [host-a]: FAILED! => {"msg": "assertion failed"}\nPLAY RECAP\nhost-a : ok=6 changed=1 unreachable=0 failed=1\n'; exit 2 ;;
    esac ;;
  *) exit 91 ;;
esac
SH
  chmod +x "$FIXTURE/ops/scripts/"* "$BIN/netbird"
  printf '%s\n' '{"test_hosts":{"hosts":["host-a"]},"_meta":{"hostvars":{"host-a":{"router_role":"A"}}}}' > "$STUBS/inventory"
  printf '%s\n' 'host-a | CHANGED | rc=0 >> {"ref":"example/widget@sha256:7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57","action":"deploy"}' > "$STUBS/rows"
  : > "$STUBS/plays"
  git -C "$FIXTURE" init -q
  git -C "$FIXTURE" add -A
  git -C "$FIXTURE" -c user.name=t -c user.email=t@t -c commit.gpgsign=false commit -q -m fixture
}

deploy() {
  cd "$FIXTURE" || return
  run env PATH="$BIN:$PATH" STUBS="$STUBS" GOAT_DEPLOY_LOG_DIR="$BATS_TEST_TMPDIR" GOAT_DOOR_RETRY_PAUSE=0 \
    GOAT_FRESH_SOURCE_TRUNK="$(git -C "$FIXTURE" rev-parse HEAD)" "$@" \
    /bin/bash ./ops/scripts/deploy-component.sh widget --goatnet test_site
}
calls() { wc -l < "$STUBS/$1" | tr -d ' '; }

@test "the door is opened at start, before the play and before the ledgers are read" {
  echo ok > "$STUBS/plays"
  deploy
  [ "$status" -eq 0 ]
  [[ "$output" == *"every host's ledger names the declared ref"* ]]
  [ "$(calls door-calls)" -eq 3 ]
}

@test "a door that fails to open once is tried again" {
  echo ok > "$STUBS/plays"
  echo 1 > "$STUBS/door-fail"
  deploy
  [ "$status" -eq 0 ]
  [[ "$output" == *"every host's ledger names the declared ref"* ]]
}

@test "a door that never opens stops the run before any play" {
  echo 99 > "$STUBS/door-fail"
  deploy
  [ "$status" -ne 0 ]
  [[ "$output" == *"did not open (tried twice)"* ]]
  [ ! -e "$STUBS/play-calls" ]
}

@test "a play that lost its connection runs once more and converges" {
  printf 'unreachable\nok\n' > "$STUBS/plays"
  deploy
  [ "$status" -eq 0 ]
  [[ "$output" == *"the connection dropped"* ]]
  [[ "$output" == *"every host's ledger names the declared ref"* ]]
  [ "$(calls play-calls)" -eq 2 ]
  ls "$BATS_TEST_TMPDIR"/goat-deploy-widget-test_hosts-*.log.first
}

@test "a second dropped connection is a failed deploy, with no third run" {
  printf 'unreachable\nunreachable\nok\n' > "$STUBS/plays"
  deploy
  [ "$status" -ne 0 ]
  [[ "$output" == *"deploy FAILED"* ]]
  [ "$(calls play-calls)" -eq 2 ]
}

@test "a failed task is not retried" {
  printf 'failed\nok\n' > "$STUBS/plays"
  deploy
  [ "$status" -ne 0 ]
  [[ "$output" == *"deploy FAILED"* ]]
  [[ "$output" != *"the connection dropped"* ]]
  [ "$(calls play-calls)" -eq 1 ]
}

@test "GOAT_DEPLOY_TRANSPORT_RETRY=0 turns the retry off" {
  printf 'unreachable\nok\n' > "$STUBS/plays"
  deploy GOAT_DEPLOY_TRANSPORT_RETRY=0
  [ "$status" -ne 0 ]
  [ "$(calls play-calls)" -eq 1 ]
}
