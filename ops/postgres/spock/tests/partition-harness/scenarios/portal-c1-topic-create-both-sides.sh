# shellcheck shell=bash disable=SC2034  # sourced by ../run.sh, which reads EXPECT
# Row P19. Class 1: new rows with site-unique or derived ids.
# Pass beyond the common conditions: both sites register the same topic while
# cut off, neither is refused, and after heal there is one topics row for it,
# under one id, at both sites (inventory §10: "One topics row").
#
# Driven by: SQL. The write is CreateTopic (ops/goat-zenoh-portal/internal/
# registry/registry.go) for `POST /api/orgs/{vendor}/topics`: one topics row
# whose id comes from the table's identity sequence; a unique violation is
# reported to the caller as "topic exists". The seed is the org and one topic
# registered before the cut, as any goatnet's catalogue already holds: it
# reaches B by replication, which does not advance B's sequence.
#
# Design §3.1 class 1: topics.id becomes derived from (vendor_prefix,
# key_pattern), so the two registrations are one row and B's sequence never
# hands out an id A already used.

V="h$(xid | cut -c1-9)"
EXPECT=""

seed() {
  portal_vendor a "$V"
  portal_topic a "$V" "$V/sys/existing/v1" > "$SCRATCH/seed"
}
write_a() { portal_topic a "$V" "$V/sys/new/v1" > "$SCRATCH/a"; }
write_b() { portal_topic b "$V" "$V/sys/new/v1" > "$SCRATCH/b"; }

check() {
  echo "    seed topic id at A: $(cat "$SCRATCH/seed")"
  expect_eq "a: registering $V/sys/new/v1" "$(sed 's/^[0-9][0-9]*$/registered/' "$SCRATCH/a")" registered
  expect_eq "b: registering $V/sys/new/v1" "$(sed 's/^[0-9][0-9]*$/registered/' "$SCRATCH/b")" registered
  local ids=()
  for s in a b; do
    expect_eq "$s: topics rows for $V/sys/new/v1" \
      "$(pr "$s" "SELECT count(*) FROM topics WHERE vendor_prefix='$V' AND key_pattern='$V/sys/new/v1';")" 1
    ids+=("$(pr "$s" "SELECT coalesce(string_agg(id::text, ',' ORDER BY id), '') FROM topics WHERE vendor_prefix='$V' AND key_pattern='$V/sys/new/v1';")")
  done
  expect_eq "its id at B" "${ids[1]}" "${ids[0]}"
}
