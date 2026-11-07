#!/usr/bin/env bats

@test "newsite secrets persist references, preserve existing values and fail closed" {
  run python3 "$BATS_TEST_DIRNAME/test_newsite_secrets.py"
  [ "$status" -eq 0 ]
}

@test "live P12 generator coverage proves a missing exact family fails" {
  # The coverage contract reads one goatnet's own OIDC-client play.
  ls "$BATS_TEST_DIRNAME"/../ops/ansible/*-prod-oidc-clients.yml >/dev/null 2>&1 \
    || skip "no goatnet's OIDC-client play is in this tree"
  run "$BATS_TEST_DIRNAME/../ops/ci/check-newsite-secrets-coverage.sh" --self-test
  [ "$status" -eq 0 ]
}
