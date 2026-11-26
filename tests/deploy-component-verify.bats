#!/usr/bin/env bats
# Ledger verification in deploy-component.sh refuses anything short of proof
# (#1545): a failed inventory or ledger read, a host that never answers, a
# failed deploy command, or a run that verified nothing. Every host and
# credential command is stubbed; nothing here contacts a goatnet.

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
  # The intent check (GB-383) is tested by crates/goatbuilder/tests/deploy_dry_run_refusals.rs.
  printf 'print("ok: signature valid")\n' > "$FIXTURE/ops/pins/generate-pins.py"
  echo 'widget_image: "example/widget@sha256:7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57"' > "$FIXTURE/ops/ansible/group_vars/test_hosts/pins.yml"
  echo 'hosts: "{{ target }}"' > "$FIXTURE/ops/ansible/widget-deploy.yml"
  printf '#!/usr/bin/env bash\nexit 1\n' > "$FIXTURE/ops/scripts/env-door.sh"
  printf '#!/usr/bin/env bash\nprintf "Profile: x\\nManagement: Disconnected\\n"\n' > "$BIN/netbird"
  # with-aws is the wrapper every host command goes through.
  cat > "$FIXTURE/ops/scripts/with-aws" <<'SH'
#!/usr/bin/env bash
shift 2
case "$1" in
  ansible-inventory) cat "$STUBS/inventory"; exit "${INVENTORY_RC:-0}" ;;
  ansible) cat "$STUBS/rows"; exit "${LEDGER_RC:-0}" ;;
  ../scripts/ansible-preflight.sh)
    printf 'PLAY RECAP\nhost-a : ok=1 changed=1 unreachable=0 failed=0\nhost-b : ok=1 changed=1 unreachable=0 failed=0\n'
    exit "${DEPLOY_RC:-0}" ;;
  *) exit 91 ;;
esac
SH
  chmod +x "$FIXTURE/ops/scripts/"* "$BIN/netbird"
  printf '%s\n' '{"test_hosts":{"hosts":["host-a","host-b"]},"_meta":{"hostvars":{"host-a":{"router_role":"A"},"host-b":{"router_role":"B"}}}}' > "$STUBS/inventory"
  good_rows > "$STUBS/rows"
}

row() { printf '%s | CHANGED | rc=0 >> {"ref":"example/widget@sha256:%s","action":"%s"}\n' "$1" "${2:-7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57}" "${3:-deploy}"; }
good_rows() { row host-a; row host-b; }

single_writer() {
  local pins="$FIXTURE/ops/pins/fleet-pins.yaml"
  awk '/^    var: widget_image$/ {print; print "    single_homed_role: A"; next} {print}' "$pins" > "$pins.new"
  mv "$pins.new" "$pins"
}

one_host() { # $1: the YAML value of the group variable that names the host
  local pins="$FIXTURE/ops/pins/fleet-pins.yaml"
  awk '/^    var: widget_image$/ {print; print "    one_host_var: widget_host"; next} {print}' "$pins" > "$pins.new"
  mv "$pins.new" "$pins"
  printf 'widget_host: %s\n' "$1" > "$FIXTURE/ops/ansible/group_vars/test_hosts/widget.yml"
}

verify() {
  cd "$FIXTURE" || return
  run env PATH="$BIN:$PATH" STUBS="$STUBS" GOAT_DEPLOY_LOG_DIR="$BATS_TEST_TMPDIR" \
    /bin/bash ./ops/scripts/deploy-component.sh widget --goatnet test_site --verify-only
}

# A real run first proves the checkout current with the trunk (contract C3);
# the fixture is its own trunk, so these runs reach the deploy and verify steps.
real_deploy() {
  git -C "$FIXTURE" init -q
  git -C "$FIXTURE" add -A
  git -C "$FIXTURE" -c user.name=t -c user.email=t@t -c commit.gpgsign=false commit -q -m fixture
  cd "$FIXTURE" || return
  run env PATH="$BIN:$PATH" STUBS="$STUBS" GOAT_DEPLOY_LOG_DIR="$BATS_TEST_TMPDIR" \
    GOAT_FRESH_SOURCE_TRUNK="$(git -C "$FIXTURE" rev-parse HEAD)" \
    /bin/bash ./ops/scripts/deploy-component.sh widget --goatnet test_site
  [[ "$output" != *"not current with the source of record"* ]]
}

refused() {
  [ "$status" -ne 0 ]
  [[ "$output" != *"every host's ledger names the declared ref"* ]]
}

@test "verify accepts every expected host with a completed deploy" {
  verify
  [ "$status" -eq 0 ]
  [[ "$output" == *"host-a: converged on sha256:7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57"* ]]
  [[ "$output" == *"host-b: converged on sha256:7e577e577e577e577e577e577e577e577e577e577e577e577e577e577e577e57"* ]]
  [[ "$output" == *"every host's ledger names the declared ref"* ]]
}

@test "verify refuses empty ledger output" {
  : > "$STUBS/rows"
  verify
  refused
  [[ "$output" == *"host-a: NOT converged — no ledger row came back"* ]]
}

@test "verify refuses a host that never answers" {
  row host-a > "$STUBS/rows"
  verify
  refused
  [[ "$output" == *"host-b: NOT converged — no ledger row came back"* ]]
}

@test "verify refuses a failed ledger read even with converged rows" {
  export LEDGER_RC=4
  verify
  refused
  [[ "$output" == *"ledger read FAILED (exit 4)"* ]]
}

@test "verify refuses a failed inventory read" {
  export INVENTORY_RC=1
  verify
  refused
  [[ "$output" == *"inventory read FAILED"* ]]
}

@test "verify refuses an unreadable inventory" {
  echo 'not json' > "$STUBS/inventory"
  verify
  refused
}

@test "verify refuses a run that verified nothing" {
  echo '{"test_hosts":{"hosts":[]}}' > "$STUBS/inventory"
  : > "$STUBS/rows"
  verify
  refused
  [[ "$output" == *"nothing was verified"* ]]
}

@test "verify includes hosts in child groups" {
  echo '{"test_hosts":{"children":["child"]},"child":{"hosts":["host-a","host-b"]}}' > "$STUBS/inventory"
  row host-a > "$STUBS/rows"
  verify
  refused
  [[ "$output" == *"host-b: NOT converged — no ledger row came back"* ]]
}

@test "verify refuses pending actions and wrong digests" {
  for bad in "$(row host-a test pending)" "$(row host-a wrong deploy)"; do
    { printf '%s\n' "$bad"; row host-b; } > "$STUBS/rows"
    verify
    refused
  done
}

@test "verify skips only the non-writer of a single-homed component" {
  single_writer
  row host-a > "$STUBS/rows"
  verify
  [ "$status" -eq 0 ]
  [[ "$output" == *"not expected on: host-b"* ]]
  [[ "$output" == *"host-a: converged"* ]]
}

@test "verify refuses a missing writer even when another host reports success" {
  single_writer
  row host-b > "$STUBS/rows"
  verify
  refused
  [[ "$output" == *"host-a: NOT converged — no ledger row came back"* ]]
}

@test "verify expects only the host a one-host component's group variable names" {
  one_host host-b
  row host-b > "$STUBS/rows"
  verify
  [ "$status" -eq 0 ]
  [[ "$output" == *"test_hosts: on one host, widget_host=host-b — not expected on: host-a"* ]]
  [[ "$output" == *"host-b: converged"* ]]
}

@test "verify refuses a one-host component whose host has no row, whatever the others say" {
  one_host host-b
  row host-a > "$STUBS/rows"
  verify
  refused
  [[ "$output" == *"host-b: NOT converged — no ledger row came back"* ]]
}

@test "verify refuses a one-host component whose host cannot be read from the checkout" {
  for value in '"{{ groups.x[0] }}"' '""' 'host-z'; do
    one_host "$value"
    good_rows > "$STUBS/rows"
    verify
    refused
    [[ "$output" == *"test_hosts: inventory could not be read — widget_host for test_hosts"* ]]
    awk '!/one_host_var: widget_host/' "$FIXTURE/ops/pins/fleet-pins.yaml" > "$FIXTURE/ops/pins/p.new" && mv "$FIXTURE/ops/pins/p.new" "$FIXTURE/ops/pins/fleet-pins.yaml"
  done
}

@test "verify treats a host with no router_role as A, as before" {
  single_writer
  echo '{"test_hosts":{"hosts":["host-a","host-b"]},"_meta":{"hostvars":{"host-b":{"router_role":"B"}}}}' > "$STUBS/inventory"
  row host-a > "$STUBS/rows"
  verify
  [ "$status" -eq 0 ]
}

@test "post-deploy verification accepts complete results" {
  real_deploy
  [ "$status" -eq 0 ]
  [[ "$output" == *"every host's ledger names the declared ref"* ]]
}

@test "post-deploy verification refuses partial results" {
  row host-a > "$STUBS/rows"
  real_deploy
  refused
}

@test "post-deploy refuses a failed deploy command despite a clean recap" {
  export DEPLOY_RC=2
  real_deploy
  refused
  [[ "$output" == *"test_hosts: deploy FAILED"* ]]
}

@test "a host that answered with the wrong row is reported once, not as missing" {
  { row host-a test pending; row host-b; } > "$STUBS/rows"
  verify
  refused
  [[ "$output" == *"host-a: NOT converged"* ]]
  [[ "$output" != *"host-a: NOT converged — no ledger row came back"* ]]
}
