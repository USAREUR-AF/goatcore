#!/usr/bin/env bats
# SPDX-License-Identifier: Apache-2.0
#
# ansible-preflight.sh refuses a direct play that would put a fleet component's
# ref on a rung above the lowest before the rung below pins it (ADR 1120,
# GB-373), including when the ref comes from an -e override of the image
# variable. The guard is loaded out of the real script so the test cannot drift.

setup() {
  command -v python3 >/dev/null || skip "python3 not installed"
  python3 -c 'import yaml' 2>/dev/null || skip "PyYAML not installed"
  SCRIPT="$BATS_TEST_DIRNAME/../ops/scripts/ansible-preflight.sh"
  GUARD="$BATS_TEST_TMPDIR/guard.sh"
  sed -n '/^goat_ladder_gate() {/,/^}/p' "$SCRIPT" > "$GUARD"
  [ -s "$GUARD" ]
  R="$BATS_TEST_TMPDIR/repo"
  mkdir -p "$R/ops/release" "$R/ops/pins"
  for g in zagros_dev_hosts zagros_hosts zagros_prod_hosts; do mkdir -p "$R/ops/ansible/group_vars/$g"; done
  cp "$BATS_TEST_DIRNAME/../ops/release/ladder_gate.py" "$R/ops/release/"
  cat > "$R/ops/pins/fleet-pins.yaml" <<'YAML'
goatnets:
  zagros_dev: {members: [zagros_dev_hosts]}
  zagros: {members: [zagros_hosts]}
  zagros_prod: {members: [zagros_prod_hosts]}
components:
  app: {var: app_image, fleet: "example/app@sha256:aaaa", deploy_playbook: app-deploy.yml}
YAML
  cat > "$R/ops/release/ladder.yaml" <<'YAML'
rungs:
  - {name: dev, goatnet: zagros_dev, status: live}
  - {name: preprod, goatnet: zagros, status: live}
  - {name: prod, goatnet: zagros_prod, status: live}
YAML
  printf 'app_image: "example/app@sha256:bbbb"\n' > "$R/ops/ansible/group_vars/zagros_dev_hosts/pins.yml"
  printf 'app_image: "example/app@sha256:aaaa"\n' > "$R/ops/ansible/group_vars/zagros_hosts/pins.yml"
  printf 'app_image: "example/app@sha256:bbbb"\n' > "$R/ops/ansible/group_vars/zagros_prod_hosts/pins.yml"
}

run_gate() {
  run env -i PATH="$PATH" REPO_ROOT="$R" "$@" bash -c '. "$1"; shift; goat_ladder_gate "$@"; echo passed' _ "$GUARD" "${ARGS[@]}"
}

@test "the gate is called before the staleness bypass" {
  run grep -c '^goat_ladder_gate "\$@"$' "$SCRIPT"
  [ "$output" = 1 ]
  gate_line=$(grep -n '^goat_ladder_gate "\$@"$' "$SCRIPT" | cut -d: -f1)
  bypass_line=$(grep -n 'GOAT_SKIP_ANSIBLE_PREFLIGHT:-}" = "1"' "$SCRIPT" | head -1 | cut -d: -f1)
  [ "$gate_line" -lt "$bypass_line" ]
}

@test "a prod play whose ref preprod does not pin is refused" {
  ARGS=(-i inv.ini app-deploy.yml -e target=zagros_prod_hosts)
  run_gate
  [ "$status" -ne 0 ]
  [[ "$output" == *"GB-373"* ]]
  [[ "$output" != *passed* ]]
}

@test "an -e override of the image variable is what is judged" {
  ARGS=(app-deploy.yml -e target=zagros_prod_hosts -e app_image=example/app@sha256:aaaa)
  run_gate
  [ "$status" -eq 0 ]
  [[ "$output" == *passed* ]]
}

@test "the skip needs both a reason and an ack, and then passes" {
  ARGS=(app-deploy.yml -e target=zagros_prod_hosts)
  run_gate GOAT_LADDER_SKIP_REASON="prod is down"
  [ "$status" -ne 0 ]
  [[ "$output" == *"--ack"* ]]
  run_gate GOAT_LADDER_SKIP_REASON="prod is down" GOAT_LADDER_SKIP_ACK=operator
  [ "$status" -eq 0 ]
  [[ "$output" == *SKIPPED* ]]
}

@test "a run deploy-component.sh already gated is not gated twice" {
  ARGS=(app-deploy.yml -e target=zagros_prod_hosts)
  run_gate GOAT_LADDER_CHECKED=1
  [ "$status" -eq 0 ]
  [[ "$output" == *passed* ]]
}
