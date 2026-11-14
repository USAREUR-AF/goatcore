# shellcheck shell=bash disable=SC2034  # sourced by ../run.sh, which reads EXPECT
# ADR 1205 D4, override 2: the hub severs a site. After the hub's timer runs,
# A applies nothing B writes. B's later key never reaches A. Restore is a
# re-seed, never a replay: B's copy is reset to the hub's, A's subscription
# from B is re-created (a fresh slot, so B's queued changes are dropped), and
# a restore_site directive re-enables it.

after_heal() {
  local db d
  d=$(directive a sever_site site_b); echo "$d" > "$SCRATCH/sever"
  apply_directives a
  for db in "${DBS[@]}"; do
    expect_eq "a: $db subscription from site_b enabled" \
      "$(q a "$db" "SELECT sub_enabled FROM spock.subscription WHERE sub_name = 'sub_b_to_a';")" f
  done
  # One direction only: A's slots are read by B; B's slot for A now has no reader.
  for db in "${DBS[@]}"; do
    q a "$db" "SET statement_timeout = '90s'; SELECT spock.wait_slot_confirm_lsn(NULL, NULL);" >/dev/null
  done
  apply_directives b
  xid > "$SCRATCH/key"
  nb_mint_key b "$(cat "$SCRATCH/key")" "written-after-sever"
  sleep 5
  expect_eq "a: key B wrote after the sever" "$(nb a "SELECT count(*) FROM setup_keys WHERE id = '$(cat "$SCRATCH/key")';")" 0

  # Restore by re-seed: B's post-sever write removed at B only, then a fresh subscription.
  nb b "SET session_replication_role = replica; DELETE FROM setup_keys WHERE id = '$(cat "$SCRATCH/key")';" >/dev/null
  for db in "${DBS[@]}"; do
    q a "$db" "SELECT spock.sub_drop('sub_b_to_a');" >/dev/null
  done
  for db in "${DBS[@]}"; do
    q a "$db" "SELECT spock.sub_create(
        subscription_name := 'sub_b_to_a',
        provider_dsn      := 'host=$PFX-b dbname=$db user=replicator password=$PW',
        replication_sets  := ARRAY['$("${PY[@]}" repset "$db")'],
        synchronize_data  := false,
        forward_origins   := ARRAY[]::text[]);" >/dev/null
  done
  for db in "${DBS[@]}"; do q a "$db" "SELECT spock.sub_disable('sub_b_to_a', immediate := true);" >/dev/null; done
  d=$(directive a restore_site site_b); echo "$d" > "$SCRATCH/restore"
  apply_directives a
  ts_wait_replicating "${DBS[@]}"
  xid > "$SCRATCH/key2"
  nb_mint_key b "$(cat "$SCRATCH/key2")" "written-after-restore"
}

check() {
  expect_eq "a: sever verdict"   "$(verdict a "$(cat "$SCRATCH/sever")" | cut -d' ' -f1)" applied
  expect_eq "b: sever verdict"   "$(verdict b "$(cat "$SCRATCH/sever")" | cut -d' ' -f1)" applied
  expect_eq "a: restore verdict" "$(verdict a "$(cat "$SCRATCH/restore")" | cut -d' ' -f1)" applied
  local s
  for s in a b; do
    expect_eq "$s: key written after the sever" "$(nb "$s" "SELECT count(*) FROM setup_keys WHERE id = '$(cat "$SCRATCH/key")';")" 0
    expect_eq "$s: key written after the restore" "$(nb "$s" "SELECT count(*) FROM setup_keys WHERE id = '$(cat "$SCRATCH/key2")';")" 1
  done
}
