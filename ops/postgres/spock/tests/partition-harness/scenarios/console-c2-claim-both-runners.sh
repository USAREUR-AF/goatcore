# shellcheck shell=bash disable=SC2034  # sourced by ../run.sh, which reads EXPECT
# Rows C7, C8, C18. Class 2: a proposal approved at A before the cut, so both
# sites hold it as `approved`. While cut off, each site also admits a person of
# its own (intake, sponsor approval, approval at its console), and each site's
# onboard-runner runs one cycle against its own console: it claims every
# approved proposal it sees and mints for it (goat-shim.sh: a NetBird setup key
# named for the handle, a Kratos identity for the email, at that site).
# Pass (inventory §10): one runner claims the shared proposal; one setup key and
# one identity are minted for it. Each site's own admission onboards once.
# The claim's `owner_site` filter (design §3.1, class 2) is what makes it pass;
# without it both runners win the claim on their own copy.

SHARED="c7-shared-$RUN"; OWN_A="c7-at-a-$RUN"; OWN_B="c7-at-b-$RUN"
# Once claims are owner-filtered, nothing collides: no Spock row is expected.
EXPECT=""

seed() { console_admit a "$SHARED" "$SHARED@harness.test" > "$SCRATCH/shared-id"; }
write_a() {
  console_admit a "$OWN_A" "$OWN_A@harness.test" > "$SCRATCH/own-a-id"
  run_runner a
}
write_b() {
  console_admit b "$OWN_B" "$OWN_B@harness.test" > "$SCRATCH/own-b-id"
  run_runner b
}

check() {
  local s h claimed key_rows key_named
  for s in a b; do sed "s/^/      [runner $s] /" "$SCRATCH/runner-$s.log" | grep -E 'claimed|minted|failed|error' || true; done
  claimed=$(grep -l "id=$(cat "$SCRATCH/shared-id") .*msg=claimed\|msg=claimed .*id=$(cat "$SCRATCH/shared-id")" \
              "$SCRATCH"/runner-a.log "$SCRATCH"/runner-b.log 2>/dev/null | wc -l | tr -d ' ')
  expect_eq "runners that claimed the shared proposal" "$claimed" 1
  for h in "$SHARED" "$OWN_A" "$OWN_B"; do
    for s in a b; do
      expect_eq "$s: NetBird setup keys named $h" "$(nb "$s" "SELECT count(*) FROM setup_keys WHERE name = '$h';")" 1
      expect_eq "$s: Kratos identities with $h's email" \
        "$(kr "$s" "SELECT count(*) FROM identities WHERE traits->>'email' = '$h@harness.test';")" 1
    done
  done
  for s in a b; do
    expect_eq "$s: the shared proposal" "$(cdb "$s" "SELECT status FROM onboard_proposals WHERE handle = '$SHARED';")" onboarded
    key_rows=$(cdb "$s" "SELECT setup_key FROM onboard_proposals WHERE handle = '$SHARED';")
    key_named=$(nb "$s" "SELECT string_agg(key, ',' ORDER BY key) FROM setup_keys WHERE name = '$SHARED';")
    expect_eq "$s: the console's setup key for the shared proposal is the only key minted for it" "$key_rows" "$key_named"
  done
}
