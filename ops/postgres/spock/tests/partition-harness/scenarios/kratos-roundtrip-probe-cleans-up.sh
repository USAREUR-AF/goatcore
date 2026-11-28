# shellcheck shell=bash disable=SC2034,SC2154  # sourced by ../run.sh, which reads EXPECT and sets KRATOS_ADMIN_a/_b
# ops/validation/identity-roundtrip.sh, the opt-in write leg of tutorial 2's
# checkpoint 3, against the two sites' Kratos admin APIs (A the founder, B the
# joiner).
#
# Cut off: the probe creates its identity at A, waits for it at B, gives up at
# its timeout, deletes what it created and exits 1. After heal the create and
# the delete replicate in order, so neither site holds a probe identity.
# Then, linked:
#   without --write it refuses (exit 2) and writes nothing;
#   with an address that already exists it refuses (exit 2) and leaves that
#   identity as it was;
#   with --write it passes (exit 0): created at A, seen at B, deleted at B,
#   gone at A, and neither site holds a probe identity.

EXPECT=""
PROBE="$REPO/ops/validation/identity-roundtrip.sh"
URL_A="http://127.0.0.1:$KRATOS_ADMIN_a"
URL_B="http://127.0.0.1:$KRATOS_ADMIN_b"
EX="goat-roundtrip-existing-$RUN@example.org"

probes() { # probes <site>  -> how many probe identities the site holds
  kr "$1" "SELECT count(*) FROM identities WHERE traits->>'email' LIKE 'goat-roundtrip-%';"
}
probe() { # probe <name> <args...>  -> runs the probe, keeps its output and exit code under <name>
  local name="$1"; shift
  set +e
  "$PROBE" "$@" > "$SCRATCH/$name.out" 2>&1
  echo $? > "$SCRATCH/$name.rc"
  set -e
}

write_a() {
  probe cutoff --write --founder "$URL_A" --joiner "$URL_B" --timeout 5
}

after_heal() {
  probes a > "$SCRATCH/cutoff-left-a"
  probes b > "$SCRATCH/cutoff-left-b"

  probe nowrite --founder "$URL_A" --joiner "$URL_B"
  probes a > "$SCRATCH/nowrite-left-a"

  kratos_create a "$EX" "Existing" > "$SCRATCH/ex"
  sync_all
  probe existing --write --founder "$URL_A" --joiner "$URL_B" --email "$EX"
  kr a "SELECT count(*) FROM identities WHERE id = '$(cat "$SCRATCH/ex")';" > "$SCRATCH/existing-kept"
  kapi a DELETE "/admin/identities/$(cat "$SCRATCH/ex")" >/dev/null
  sync_all

  probe pass --write --founder "$URL_A" --joiner "$URL_B" --timeout 60
  sync_all
  probes a > "$SCRATCH/pass-left-a"
  probes b > "$SCRATCH/pass-left-b"
}

check() {
  expect_eq "cut off: the probe's exit" "$(cat "$SCRATCH/cutoff.rc")" 1
  expect_eq "cut off: the probe says it did not reach the joiner" \
    "$(grep -c 'did not reach the joiner' "$SCRATCH/cutoff.out")" 1
  expect_eq "cut off: the probe says it deleted what it created" \
    "$(grep -c 'cleaned up: deleted .* at the founder' "$SCRATCH/cutoff.out")" 1
  expect_eq "a: probe identities after the cut-off run heals" "$(cat "$SCRATCH/cutoff-left-a")" 0
  expect_eq "b: probe identities after the cut-off run heals" "$(cat "$SCRATCH/cutoff-left-b")" 0
  expect_eq "without --write: the probe's exit" "$(cat "$SCRATCH/nowrite.rc")" 2
  expect_eq "without --write: it says it writes" "$(grep -c 'refused: this WRITES' "$SCRATCH/nowrite.out")" 1
  expect_eq "without --write: probe identities at a" "$(cat "$SCRATCH/nowrite-left-a")" 0
  expect_eq "an existing address: the probe's exit" "$(cat "$SCRATCH/existing.rc")" 2
  expect_eq "an existing address: it says so" "$(grep -c 'already exists at the founder; nothing was written' "$SCRATCH/existing.out")" 1
  expect_eq "an existing address: that identity is kept" "$(cat "$SCRATCH/existing-kept")" 1
  expect_eq "linked: the probe's exit" "$(cat "$SCRATCH/pass.rc")" 0
  expect_eq "linked: the probe's PASS line" "$(grep -c '^identity-roundtrip: PASS ' "$SCRATCH/pass.out")" 1
  expect_eq "a: probe identities after the linked run" "$(cat "$SCRATCH/pass-left-a")" 0
  expect_eq "b: probe identities after the linked run" "$(cat "$SCRATCH/pass-left-b")" 0
}
