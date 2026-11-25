#!/usr/bin/env bats
# SPDX-License-Identifier: Apache-2.0
#
# deploy-component.sh refuses to put a ref on a rung above the lowest before the
# rung below runs it (ADR 1120, GB-373), unless a recorded exception applies.
# Every operational command is stubbed; these tests contact no goatnet or host.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
SUBJECT="$REPO_ROOT/ops/scripts/deploy-component.sh"
OLD="example/app@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
NEW="example/app@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"

setup() {
  command -v python3 >/dev/null || skip "python3 not installed"
  python3 -c 'import yaml' 2>/dev/null || skip "PyYAML not installed"
  FIXTURE="$BATS_TEST_TMPDIR/repo"; BIN="$BATS_TEST_TMPDIR/bin"; CALLS="$BATS_TEST_TMPDIR/operational-calls"
  mkdir -p "$FIXTURE/ops/scripts" "$FIXTURE/ops/pins" "$FIXTURE/ops/release/records/app" "$BIN"
  for g in zagros_dev_hosts zagros_hosts zagros_prod_hosts; do mkdir -p "$FIXTURE/ops/ansible/group_vars/$g"; done
  cp "$SUBJECT" "$FIXTURE/ops/scripts/deploy-component.sh"; chmod +x "$FIXTURE/ops/scripts/deploy-component.sh"
  cp "$REPO_ROOT/ops/release/ladder_gate.py" "$FIXTURE/ops/release/"
  cat > "$FIXTURE/ops/pins/fleet-pins.yaml" <<YAML
goatnets:
  zagros_dev: {members: [zagros_dev_hosts]}
  zagros: {members: [zagros_hosts]}
  zagros_prod: {members: [zagros_prod_hosts]}
components:
  app: {var: app_image, fleet: "$OLD", deploy_playbook: app-deploy.yml}
YAML
  cat > "$FIXTURE/ops/release/ladder.yaml" <<'YAML'
rungs:
  - {name: dev, goatnet: zagros_dev, status: live}
  - {name: preprod, goatnet: zagros, status: live}
  - {name: prod, goatnet: zagros_prod, status: live}
hops: []
YAML
  printf 'app_image: "%s"\n' "$NEW" > "$FIXTURE/ops/ansible/group_vars/zagros_dev_hosts/pins.yml"
  printf 'app_image: "%s"\n' "$OLD" > "$FIXTURE/ops/ansible/group_vars/zagros_hosts/pins.yml"
  printf 'app_image: "%s"\n' "$NEW" > "$FIXTURE/ops/ansible/group_vars/zagros_prod_hosts/pins.yml"
  printf '%s\n' 'hosts: "{{ target }}"' > "$FIXTURE/ops/ansible/app-deploy.yml"
  for stub in netbird aws-vault nc; do
    printf '#!/usr/bin/env bash\nprintf "%s %%s\\n" "$*" >> "$OPERATIONAL_CALLS"\nexit 1\n' "$stub" > "$BIN/$stub"; chmod +x "$BIN/$stub"
  done
  printf '#!/usr/bin/env bash\nprintf "env-door %%s\\n" "$*" >> "$OPERATIONAL_CALLS"\nexit 1\n' > "$FIXTURE/ops/scripts/env-door.sh"
  chmod +x "$FIXTURE/ops/scripts/env-door.sh"
  # A dry run runs the fresh-source check too (it touches no host); here it passes.
  # The intent check (GB-383) is tested by crates/goatbuilder/tests/deploy_dry_run_refusals.rs.
  printf 'print("ok: signature valid")\n' > "$FIXTURE/ops/pins/generate-pins.py"
  printf '#!/usr/bin/env bash\nexit 0\n' > "$FIXTURE/ops/scripts/fresh-source.sh"
  chmod +x "$FIXTURE/ops/scripts/fresh-source.sh"
}

run_deploy() {
  cd "$FIXTURE" || return
  run env PATH="$BIN:$PATH" OPERATIONAL_CALLS="$CALLS" "${BASH_UNDER_TEST:-/bin/bash}" ./ops/scripts/deploy-component.sh "$@"
}

@test "a prod deploy of a ref the preprod rung does not pin is refused before any operation" {
  run_deploy app --goatnet zagros_prod --dry-run
  [ "$status" -eq 1 ]
  [[ "$output" == *"GB-373"* ]]
  [[ "$output" == *"rung preprod (goatnet zagros)"* ]]
  [[ "$output" == *"deploy-component.sh app --goatnet zagros"* ]]
  # A dry run is also a plan: it prints all of it, then exits 1.
  [[ "$output" == *"DRY: would read /var/lib/goat/app/deploy-history.jsonl on zagros_prod_hosts"* ]]
  [[ "$output" == *"DRY: a real run would refuse before any host (ladder)"* ]]

  rm -f "$CALLS"
  run_deploy app --goatnet zagros_prod
  [ "$status" -eq 1 ]
  [[ "$output" == *"GB-373"* ]]
  [ ! -s "$CALLS" ]
}

@test "the same ref is allowed once preprod pins it, and the run says it confirms convergence there" {
  printf 'app_image: "%s"\n' "$NEW" > "$FIXTURE/ops/ansible/group_vars/zagros_hosts/pins.yml"
  run_deploy app --goatnet zagros_prod --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"pinned on rung preprod (zagros)"* ]]
  [[ "$output" == *"DRY: would confirm app converged on zagros"* ]]
}

@test "--skip-rung without --reason is refused" {
  run_deploy app --goatnet zagros_prod --dry-run --skip-rung --ack operator
  [ "$status" -eq 1 ]
  [[ "$output" == *"--skip-rung needs --reason"* ]]
}

@test "--skip-rung with a reason and an ack passes and says it is recorded" {
  run_deploy app --goatnet zagros_prod --dry-run --skip-rung --reason "preprod is down" --ack operator
  [ "$status" -eq 0 ]
  [[ "$output" == *"SKIPPED"* ]]
  [[ "$output" == *"preprod is down"* ]]
}

@test "an open hotfix record for the ref is the recorded exception" {
  printf 'component: app\nclass: hotfix\ncandidate_ref: "%s"\nstate: candidate\n' "$NEW" \
    > "$FIXTURE/ops/release/records/app/20260101-hotfix-bbbb.yaml"
  run_deploy app --goatnet zagros_prod --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"open hotfix"* ]]
}

@test "the lowest rung deploys without a gate" {
  run_deploy app --goatnet zagros_dev --dry-run
  [ "$status" -eq 0 ]
  [[ "$output" == *"lowest rung"* ]]
}
