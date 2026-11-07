#!/usr/bin/env bats
# Regression test for GH #1113.
# Asserts that the peerview lifecycle sweep NetBird API default no longer
# hard-codes an extra `netbird.` prefix, and follows the public_mgmt_fqdn
# convention used by other roles.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
DEFAULTS="$REPO_ROOT/ops/ansible/roles/peerview/defaults/main.yml"

# This tree may not carry the code a test reads (a public goatcore checkout
# does not carry a goatnet's own scripts); such a test is skipped by name,
# and runs wherever that code is present.
setup() {
  [ -f "$DEFAULTS" ] || skip "the peerview role is not in this tree"
}

@test "lifecycle sweep NetBird API default does not begin with https://netbird." {
  run grep -E '^lifecycle_sweep_nb_api:' "$DEFAULTS"
  [ "$status" -eq 0 ]
  [[ "$output" != "lifecycle_sweep_nb_api: \"https://netbird."* ]]
}

@test "lifecycle sweep NetBird API default uses public_mgmt_fqdn | default(site_dns_zone)" {
  run grep -E '^lifecycle_sweep_nb_api:' "$DEFAULTS"
  [ "$status" -eq 0 ]
  [ "$output" = 'lifecycle_sweep_nb_api: "https://{{ public_mgmt_fqdn | default(site_dns_zone) }}"' ]
}
