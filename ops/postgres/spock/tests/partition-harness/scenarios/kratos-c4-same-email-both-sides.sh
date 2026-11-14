# shellcheck shell=bash disable=SC2034  # sourced by ../run.sh, which reads EXPECT
# Rows C41, K1, K9. Class 4: the same email created at both sites while cut
# off. Spock logs the collision (the resolution queue, ADR 1165 D4). The hub
# resolves it (ADR 1205 D7): A's identity survives; identity_delete of B's is
# carried out by B, then identity_resave of A's by A, which re-issues A's
# credential rows so they reach B, which discarded them at heal.
# Pass beyond the common conditions: one identity with the email at both
# sites, and it is A's.

EMAIL="c4-$RUN@harness.test"
CONTROL_EMAIL="c4-control-$RUN@harness.test"
PASSWORD_A="C4-Survivor-Fixture!82"
PASSWORD_B="C4-Losing-Fixture!93"
CONTROL_PASSWORD="C4-Unrelated-Fixture!74"
# The collision itself, at each site (the resolution queue, ADR 1165 D4), and
# the resolution's deletes of rows that site had already discarded.
EXPECT="a kratos exception identities INSERT:other 1
a kratos exception identity_credential_identifiers INSERT:unique 1
a kratos exception identity_credentials INSERT:other 1
a kratos exception identity_recovery_addresses INSERT:unique 1
a kratos exception identity_verifiable_addresses INSERT:unique 1
a kratos resolution identity_credential_identifiers delete_delete:skip 1
a kratos resolution identity_recovery_addresses delete_delete:skip 1
a kratos resolution identity_verifiable_addresses delete_delete:skip 1
b kratos exception identities INSERT:other 1
b kratos exception identity_credential_identifiers INSERT:unique 1
b kratos exception identity_credentials INSERT:other 1
b kratos exception identity_recovery_addresses INSERT:unique 1
b kratos exception identity_verifiable_addresses INSERT:unique 1
b kratos resolution identity_credential_identifiers delete_delete:skip 1
b kratos resolution identity_recovery_addresses delete_delete:skip 1
b kratos resolution identity_verifiable_addresses delete_delete:skip 1"

seed() {
  kratos_create a "$CONTROL_EMAIL" "C4 unrelated identity" "$CONTROL_PASSWORD" > "$SCRATCH/control-id"
  sync_all
  for s in a b; do
    expect_eq "$s: unrelated identity logs in before collision" \
      "$(kratos_login "$s" "$CONTROL_EMAIL" "$CONTROL_PASSWORD")" "$(cat "$SCRATCH/control-id")"
  done
}
write_a() {
  kratos_create a "$EMAIL" "C4 at A" "$PASSWORD_A" > "$SCRATCH/id-a"
  expect_eq "a: original credential works while isolated" \
    "$(kratos_login a "$EMAIL" "$PASSWORD_A")" "$(cat "$SCRATCH/id-a")"
}
write_b() {
  kratos_create b "$EMAIL" "C4 at B" "$PASSWORD_B" > "$SCRATCH/id-b"
  expect_eq "b: original credential works while isolated" \
    "$(kratos_login b "$EMAIL" "$PASSWORD_B")" "$(cat "$SCRATCH/id-b")"
}
after_heal() {
  dump_logs "$SCRATCH/collision.logs"
  directive a identity_delete "$(cat "$SCRATCH/id-b")" > /dev/null
  directive a identity_resave "$(cat "$SCRATCH/id-a")" > /dev/null
  sync_all
  apply_directives b
  sync_all
  apply_directives a
}

check() {
  local emails s
  emails=$(grep -h -o '[a-z0-9.-]*@harness.test' "$SCRATCH/collision.logs" | sort -u | paste -sd, -)
  expect_eq "collisions logged name one email" "$emails" "$EMAIL"
  for s in a b; do
    expect_eq "$s: identities with the email" \
      "$(kr "$s" "SELECT string_agg(id::text, ',') FROM identities WHERE traits->>'email' = '$EMAIL';")" "$(cat "$SCRATCH/id-a")"
    expect_eq "$s: survivor password logs into the selected identity" \
      "$(kratos_login "$s" "$EMAIL" "$PASSWORD_A")" "$(cat "$SCRATCH/id-a")"
    expect_eq "$s: losing password cannot log into the survivor" \
      "$(kratos_login "$s" "$EMAIL" "$PASSWORD_B")" invalid_credentials
    expect_eq "$s: unrelated identity remains usable" \
      "$(kratos_login "$s" "$CONTROL_EMAIL" "$CONTROL_PASSWORD")" "$(cat "$SCRATCH/control-id")"
  done
}
