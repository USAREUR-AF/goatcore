#!/usr/bin/env bats
# SPDX-License-Identifier: Apache-2.0
#
# ansible-preflight.sh refuses to start a play while any environment value is
# still an op://, bao:// or bw:// reference, and names the variable without its
# value. The guard is loaded out of the real script so the test cannot drift.

setup() {
  SCRIPT="$BATS_TEST_DIRNAME/../ops/scripts/ansible-preflight.sh"
  GUARD="$BATS_TEST_TMPDIR/guard.sh"
  sed -n '/^goat_unresolved_reference_guard() {/,/^}/p' "$SCRIPT" > "$GUARD"
  [ -s "$GUARD" ]
}

run_guard() {
  run env -i PATH="$PATH" "$@" bash -c '. "$1"; goat_unresolved_reference_guard; echo passed' _ "$GUARD"
}

@test "the guard is called before the play starts" {
  run grep -c '^goat_unresolved_reference_guard$' "$SCRIPT"
  [ "$output" = 1 ]
  guard_line=$(grep -n '^goat_unresolved_reference_guard$' "$SCRIPT" | cut -d: -f1)
  bypass_line=$(grep -n 'GOAT_SKIP_ANSIBLE_PREFLIGHT:-}" = "1"' "$SCRIPT" | head -1 | cut -d: -f1)
  [ "$guard_line" -lt "$bypass_line" ]
}

@test "resolved values pass" {
  run_guard NETVIS_COOKIE=abcdefabcdefabcd PLAIN=value
  [ "$status" -eq 0 ]
  [[ "$output" == *passed* ]]
}

@test "an op:// value is refused, by name, without its value" {
  run_guard SITE_PROXY_COOKIE=op://vault-x/item-y/field-z
  [ "$status" -eq 2 ]
  [[ "$output" == *SITE_PROXY_COOKIE* ]]
  [[ "$output" != *vault-x* ]]
  [[ "$output" != *passed* ]]
}

@test "bao:// and bw:// values are refused too" {
  run_guard A_TOKEN=bao://kv/a B_TOKEN=bw://item/b
  [ "$status" -eq 2 ]
  [[ "$output" == *A_TOKEN* ]]
  [[ "$output" == *B_TOKEN* ]]
}

@test "an inherited GOAT_SECRETS_RESOLVED does not let a reference through" {
  run_guard GOAT_SECRETS_RESOLVED=1 CRAWLER_BUDGET_TOKEN=op://v/i/f
  [ "$status" -eq 2 ]
  [[ "$output" == *CRAWLER_BUDGET_TOKEN* ]]
}

@test "the scheme prefix setting itself is not a reference" {
  run_guard GOAT_SECRET_REF_PREFIX=bao://
  [ "$status" -eq 0 ]
}
