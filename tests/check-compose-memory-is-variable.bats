#!/usr/bin/env bats

# Tests for ops/ci/check-compose-memory-is-variable.sh.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
SUBJECT="$REPO_ROOT/ops/ci/check-compose-memory-is-variable.sh"

@test "self-test goes red on a literal and green on variables" {
  run "${BASH_UNDER_TEST:-/bin/bash}" -u "$SUBJECT" --self-test
  [ "$status" -eq 0 ]
  [[ "$output" == *"OK: self-test"* ]]
}

@test "a literal memory limit in a role template is refused with file and line" {
  root="$BATS_TEST_TMPDIR/tree"
  mkdir -p "$root/ops/ansible/roles/r/templates"
  printf 'services:\n  a:\n    deploy:\n      resources:\n        limits:\n          memory: 2G\n' \
    >"$root/ops/ansible/roles/r/templates/docker-compose.yml.j2"
  CHECK_ROOT="$root" run "${BASH_UNDER_TEST:-/bin/bash}" -u "$SUBJECT"
  [ "$status" -eq 1 ]
  [[ "$output" == *"ops/ansible/roles/r/templates/docker-compose.yml.j2:6"* ]]
  [[ "$output" == *"is a literal"* ]]
}

@test "a mgmt-host key that its .env writer does not write is refused" {
  root="$BATS_TEST_TMPDIR/tree2"
  mkdir -p "$root/ops/compose/mgmt-host" "$root/ops/ansible/tasks"
  printf 'services:\n  p:\n    deploy:\n      resources:\n        limits:\n          memory: "${P_MEMORY:-2G}"\n' \
    >"$root/ops/compose/mgmt-host/docker-compose.yml"
  printf -- '- name: x\n' >"$root/ops/ansible/mgmt-host.yml"
  printf -- '- name: x\n' >"$root/ops/ansible/tasks/mgmt-host-ceilings-env.yml"
  CHECK_ROOT="$root" run "${BASH_UNDER_TEST:-/bin/bash}" -u "$SUBJECT"
  [ "$status" -eq 1 ]
  [[ "$output" == *"does not write P_MEMORY"* ]]
}

@test "this repository passes" {
  run "${BASH_UNDER_TEST:-/bin/bash}" -u "$SUBJECT"
  [ "$status" -eq 0 ]
}
