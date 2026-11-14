# shellcheck shell=bash disable=SC2034  # sourced by ../run.sh, which reads EXPECT
# Row P19. Class 1, the loss a heal can hide.
# Pass beyond the common conditions: where last-write-wins replaced a row on
# heal, drift-watch reports a loss for that table at each site, although the
# two sites hold the same rows and its compare reports them equal.
#
# Driven by: SQL. Each site registers a different topic of one org while cut
# off (CreateTopic, ops/goat-zenoh-portal/internal/registry/registry.go). On
# the step A schema both take the next id from their own sequence, the same
# id, so on heal one row replaces the other at both sites and Spock logs it in
# spock.resolutions (insert_exists, with the two rows' content different).
# B writes last, so B's topic is the one kept. Both sequences are set to one
# value first, as on a goatnet whose sites have registered nothing since
# their sequences last agreed: the scenarios before this one leave the two
# sites' sequences apart, and the case must not depend on the order they run.
#
# This scenario holds while topics.id is a per-site sequence. Once ids are
# derived (design §3.1 class 1) two different topics no longer collide, and it
# needs another table that can still lose a row this way.

V="h$(xid | cut -c1-9)"
NEXT="$RUN"  # above every id a scenario uses; a soak pass gets a new RUN
same_next() { pr "$1" "SELECT setval(pg_get_serial_sequence('topics', 'id'), $NEXT, false);" >/dev/null; }
EXPECT="a portal_registry resolution topics insert_exists:apply_remote 1
b portal_registry resolution topics insert_exists:keep_local 1"

seed()    { portal_vendor a "$V"; }
write_a() { same_next a; portal_topic a "$V" "$V/sys/a/v1" > "$SCRATCH/a"; }
write_b() { same_next b; portal_topic b "$V" "$V/sys/b/v1" > "$SCRATCH/b"; }

check() {
  echo "    a got id $(cat "$SCRATCH/a"), b got id $(cat "$SCRATCH/b")"
  for s in a b; do
    echo "    $s holds: $(pr "$s" "SELECT string_agg(id || '=' || key_pattern, ' ' ORDER BY id) FROM topics WHERE vendor_prefix='$V';")"
  done
  local s n
  for s in a b; do
    n=$(drift_lost "site_$s" portal_registry topics)
    if [[ "$n" -ge 1 ]]; then ok "$s: drift-watch reports $n topics row(s) replaced on heal"
    else fail "$s: drift-watch reports no topics row replaced on heal (got '$n'), but one topic was lost"; fi
  done
}
