#!/usr/bin/env bash
# shellcheck disable=SC2034  # PFX, NET, PW are read by ../lib/two-site.sh
# How long a site may be off the network before the slot its provider keeps for
# it is invalidated, and what the site sees when it returns
# (docs/design/phone-node-offline-window.md).
#
# Two postgres-spock sites replicate one table both ways, as a founder (A) and
# a phone node (B) do. B is taken off the network (docker network disconnect:
# it keeps running and takes its own writes, as a phone off its uplink does).
# A writes replicated rows B misses, then makes WAL at a stated rate into a
# table it does not replicate (any WAL counts against a slot), and the slot A
# keeps for B is read every second until Postgres invalidates it
# (wal_status = lost). B then returns, by the network (RETURN=network) or by
# a restart (RETURN=restart), and the run records what each site reports until
# B's subscription stops reading 'replicating'. Last, B is seeded again the way
# the design says (a new subscription, then each table copied again after a
# local truncate), and the run checks that both sites hold the same rows and
# that the slot B keeps for A survived the copy.
#
# Every reading goes to $OUT (default: a new temporary folder):
#   samples.tsv   one row a second: seconds since the cut, WAL bytes A wrote
#                 since the cut, the slot's wal_status, safe_wal_size, the WAL
#                 the slot holds, and whether it is active
#   events.txt    the cap, the rate asked for and the rate made, the times and
#                 byte counts at which the slot went unreserved and lost, and
#                 what each site reported after B returned and after the re-seed
#   logs/         each site's Postgres log
#
#   IMAGE        postgres-spock image built from ops/postgres/spock (required)
#   RATE_MIB     WAL to make on A each second, in MiB (default 16)
#   MAX_FACTOR   stop when A has written this many caps without the slot being
#                invalidated (default 4): that is a failure, not a pass
#   RETURN       network (default) or restart: how B comes back
#   RETURN_WAIT  seconds to wait after B returns for its subscription from A to
#                stop reading 'replicating' (default 900)
#   HARNESS_PFX / HARNESS_NET / HARNESS_PORT_PG_A / HARNESS_PORT_PG_B
#                names and host ports, as ../partition-harness/run.sh takes them
#   KEEP=1       leave the containers up
#
# Exit 0 when the slot was invalidated, B reported the lost slot, and the
# re-seed converged; 1 otherwise. Substrate: Linux Docker only.

set -euo pipefail
cd "$(dirname "$0")"

IMAGE="${IMAGE:?set IMAGE to a postgres-spock image}"
RATE_MIB="${RATE_MIB:-16}"
RETURN="${RETURN:-network}"
RETURN_WAIT="${RETURN_WAIT:-900}"
case "$RETURN" in network|restart) ;; *) echo "RETURN must be network or restart" >&2; exit 2 ;; esac
MAX_FACTOR="${MAX_FACTOR:-4}"
PFX="${HARNESS_PFX:-sow}"
NET="${HARNESS_NET:-slot-offline-window-net}"
PW="partition"
TS_PORT_a="${HARNESS_PORT_PG_A:-}"
TS_PORT_b="${HARNESS_PORT_PG_B:-}"
OUT="${OUT:-$(mktemp -d)}"
mkdir -p "$OUT/logs"
# shellcheck source=../lib/two-site.sh
source ../lib/two-site.sh

DB=netbird
EV="$OUT/events.txt"
: > "$EV"
ev() { echo "$*" | tee -a "$EV"; }
now() { date -u +%Y-%m-%dT%H:%M:%SZ; }

cleanup() {
  for s in a b; do docker logs "$PFX-$s" > "$OUT/logs/$s.log" 2>&1 || true; done
  [[ "${KEEP:-0}" == 1 ]] && { echo "KEEP=1, leaving $PFX-a $PFX-b up"; return; }
  ts_down
}
trap cleanup EXIT
ts_down

ev "image $IMAGE ($(docker image inspect "$IMAGE" --format '{{.Id}}'))"
ev "started $(now)"
ts_up
CAP="$(q a postgres "SELECT pg_size_bytes(current_setting('max_slot_wal_keep_size'));")"
ev "cap max_slot_wal_keep_size = $(q a postgres "SHOW max_slot_wal_keep_size;") ($CAP bytes) on A"
ev "max_wal_size = $(q a postgres "SHOW max_wal_size;"), checkpoint_timeout = $(q a postgres "SHOW checkpoint_timeout;"), wal_segment_size = $(q a postgres "SHOW wal_segment_size;")"

# One replicated table, as the roles add tables to their repset (DDL is not
# replicated: both sites).
for s in a b; do
  q "$s" "$DB" "CREATE TABLE offwin (id uuid PRIMARY KEY DEFAULT gen_random_uuid(), site text NOT NULL,
                                      pad text NOT NULL, at timestamptz NOT NULL DEFAULT now());
                CREATE TABLE offwin_load (id bigserial PRIMARY KEY, pad text NOT NULL);"
done
ts_node_create "$DB"
for s in a b; do q "$s" "$DB" "SELECT spock.repset_add_table('default', 'offwin');" >/dev/null; done
ts_sub_create "$DB" default
ts_wait_replicating "$DB"
q a "$DB" "INSERT INTO offwin (site, pad) SELECT 'a-seed', md5(g::text) FROM generate_series(1, 100) g;"
for _ in $(seq 1 60); do
  [[ "$(q b "$DB" "SELECT count(*) FROM offwin;")" == 100 ]] && break
  sleep 1
done
[[ "$(q b "$DB" "SELECT count(*) FROM offwin;")" == 100 ]] || { ev "FAIL: the seed rows never reached B"; exit 1; }
SLOT="$(q a postgres "SELECT slot_name FROM pg_replication_slots WHERE slot_name LIKE '%sub_a_to_b%';")"
[[ -n "$SLOT" ]] || { ev "FAIL: A holds no slot for B's subscription"; q a postgres "SELECT slot_name FROM pg_replication_slots;" | tee -a "$EV"; exit 1; }
ev "A's slot for B: $SLOT"

# B leaves the network. It keeps running and writes rows of its own.
docker network disconnect "$NET" "$PFX-b"
T0="$(date +%s)"
LSN0="$(q a postgres "SELECT pg_current_wal_lsn();")"
ev "cut at $(now) (A at LSN $LSN0)"
q b "$DB" "INSERT INTO offwin (site, pad) SELECT 'b-offline', md5(g::text) FROM generate_series(1, 10) g;"
ev "B wrote 10 rows while off the network"
q a "$DB" "INSERT INTO offwin (site, pad) SELECT 'a-missed', md5(g::text) FROM generate_series(1, 50) g;"
ev "A wrote 50 replicated rows B has not seen"

# Each row's pad is 8 KiB of hex from md5, which compresses badly; rows a tick
# is the rate asked for. The rate made is read from the WAL position.
ROWS=$(( RATE_MIB * 128 ))
printf 'seconds\twal_bytes\twal_status\tsafe_wal_size\tretained\tactive\n' > "$OUT/samples.tsv"
UNRESERVED_AT=""; LOST_AT=""
while :; do
  tick="$(date +%s)"
  q a "$DB" "INSERT INTO offwin_load (pad)
             SELECT (SELECT string_agg(md5(random()::text || g::text || i::text), '') FROM generate_series(1, 256) i)
             FROM generate_series(1, $ROWS) g;"
  row="$(q a postgres "SELECT pg_wal_lsn_diff(pg_current_wal_lsn(), '$LSN0'::pg_lsn)::bigint, coalesce(wal_status, ''),
                              coalesce(safe_wal_size::text, ''), coalesce(pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn)::bigint::text, ''),
                              active
                       FROM pg_replication_slots WHERE slot_name = '$SLOT';" | tr '|' '\t')"
  el=$(( $(date +%s) - T0 ))
  printf '%s\t%s\n' "$el" "$row" >> "$OUT/samples.tsv"
  wal="$(printf '%s' "$row" | cut -f1)"; st="$(printf '%s' "$row" | cut -f2)"
  if [[ "$st" == unreserved && -z "$UNRESERVED_AT" ]]; then UNRESERVED_AT="$el"; ev "unreserved after ${el}s, $wal bytes since the cut"; fi
  if [[ "$st" == lost ]]; then LOST_AT="$el"; ev "lost after ${el}s, $wal bytes since the cut"; break; fi
  if [[ "$wal" -gt $(( CAP * MAX_FACTOR )) ]]; then ev "FAIL: $wal bytes written and the slot is still '$st'"; exit 1; fi
  left=$(( tick + 1 - $(date +%s) )); [[ "$left" -gt 0 ]] && sleep "$left"
done
EL="$(( $(date +%s) - T0 ))"
WAL="$(q a postgres "SELECT pg_wal_lsn_diff(pg_current_wal_lsn(), '$LSN0'::pg_lsn)::bigint;")"
ev "rate asked $RATE_MIB MiB/s; made $(( WAL / EL / 1048576 )) MiB/s ($WAL bytes in ${EL}s)"

# B returns, and the run waits for B's subscription from A to say the slot is gone.
R_AT="$(date +%s)"
if [[ "$RETURN" == network ]]; then
  docker network connect "$NET" "$PFX-b"
else
  docker network connect "$NET" "$PFX-b"
  docker restart "$PFX-b" >/dev/null
  ts_wait_ready "$PFX-b"
fi
ev "B back ($RETURN) at $(now)"
printf 'seconds\tstatus\n' > "$OUT/after-return.tsv"
SEEN=""
while [[ $(( $(date +%s) - R_AT )) -lt "$RETURN_WAIT" ]]; do
  st="$(q b "$DB" "SELECT status FROM spock.sub_show_status() WHERE subscription_name = 'sub_a_to_b';" | tr -d '[:space:]')"
  printf '%s\t%s\n' "$(( $(date +%s) - R_AT ))" "$st" >> "$OUT/after-return.tsv"
  if [[ "$st" != replicating ]]; then SEEN="$st"; break; fi
  sleep 5
done
if [[ -n "$SEEN" ]]; then
  ev "B's subscription from A read '$SEEN' $(( $(date +%s) - R_AT ))s after B returned; until then it read 'replicating'"
else
  ev "B's subscription from A still read 'replicating' ${RETURN_WAIT}s after B returned"
fi
sleep 10
ev "-- B's log lines about the subscription from A, since it returned"
docker logs --since "$(( $(date +%s) - R_AT + 2 ))s" "$PFX-b" 2>&1 | grep -E "ERROR|FATAL" | sed -E 's/^[0-9-]+ [0-9:.]+ [A-Z]+ \[[0-9]+\] //' | sort | uniq -c | head -8 | tee -a "$EV" || true
ev "-- A's log lines about the slot"
docker logs "$PFX-a" 2>&1 | grep -E "$SLOT|invalidat" | grep -E "ERROR|invalidat" | sed -E 's/^[0-9-]+ [0-9:.]+ [A-Z]+ \[[0-9]+\] //' | sort | uniq -c | head -8 | tee -a "$EV" || true
ev "-- A's slot now"
q a postgres "SELECT slot_name, active, wal_status, restart_lsn FROM pg_replication_slots WHERE slot_name = '$SLOT';" | tee -a "$EV"
for _ in $(seq 1 30); do
  [[ "$(q a "$DB" "SELECT count(*) FROM offwin WHERE site = 'b-offline';")" == 10 ]] && break
  sleep 2
done
ev "A holds $(q a "$DB" "SELECT count(*) FROM offwin WHERE site = 'b-offline';") of B's 10 offline rows (A's own subscription from B, whose slot B kept)"
ev "B holds $(q b "$DB" "SELECT count(*) FROM offwin WHERE site = 'a-missed';") of the 50 rows A wrote while B was away"

LOST_REPORTED=0
[[ -n "$SEEN" ]] && LOST_REPORTED=1

# The re-seed: once A holds B's own writes, B drops its subscription from A,
# which drops A's lost slot, subscribes again, and copies each replicated table
# again after a local truncate (spock.sub_resync_table; the truncate is not
# replicated). A copy into tables that still hold rows is refused
# (duplicate keys), so synchronize_data on the new subscription is not enough.
ev "-- re-seed at $(now)"
R0="$(date +%s)"
q b "$DB" "SELECT spock.sub_drop('sub_a_to_b');" >/dev/null
ev "slots on A after B's sub_drop: $(q a postgres "SELECT count(*) FROM pg_replication_slots WHERE slot_name = '$SLOT';")"
q b "$DB" "SELECT spock.sub_create(
    subscription_name := 'sub_a_to_b',
    provider_dsn      := 'host=$PFX-a dbname=$DB user=replicator password=$PW',
    replication_sets  := ARRAY['default'],
    synchronize_data  := false,
    forward_origins   := ARRAY[]::text[]);" >/dev/null
st=""
for _ in $(seq 1 300); do
  st="$(q b "$DB" "SELECT status FROM spock.sub_show_status() WHERE subscription_name = 'sub_a_to_b';" | tr -d '[:space:]')"
  [[ "$st" == replicating ]] && break
  sleep 2
done
ev "B's new subscription is '$st' after $(( $(date +%s) - R0 ))s"
BLSN="$(q b postgres "SELECT pg_current_wal_lsn();")"
q b "$DB" "SELECT spock.sub_resync_table('sub_a_to_b', 'offwin', true);" >/dev/null
sync=""
for _ in $(seq 1 300); do
  sync="$(q b "$DB" "SELECT sync_status FROM spock.local_sync_status WHERE sync_relname = 'offwin';" | tr -d '[:space:]')"
  [[ "$sync" == r ]] && break
  sleep 2
done
ev "offwin copied again: sync_status '$sync' after $(( $(date +%s) - R0 ))s; the copy wrote $(q b postgres "SELECT pg_wal_lsn_diff(pg_current_wal_lsn(), '$BLSN'::pg_lsn)::bigint;") bytes of WAL on B"
ev "B's slot for A after the copy: $(q b postgres "SELECT coalesce(wal_status, '') || ' active=' || active FROM pg_replication_slots WHERE slot_name LIKE '%sub_b_to_a%';")"
ev "-- B's log lines during the re-seed"
docker logs --since "$(( $(date +%s) - R0 + 5 ))s" "$PFX-b" 2>&1 | grep -E "ERROR|FATAL" | sort | uniq -c | head -8 | tee -a "$EV" || true
q a "$DB" "INSERT INTO offwin (site, pad) VALUES ('a-after', 'after the re-seed');"
sleep 5
SUM_A="$(q a "$DB" "SELECT count(*) || ' ' || md5(string_agg(md5(id::text || site || pad), ',' ORDER BY id)) FROM offwin;")"
SUM_B="$(q b "$DB" "SELECT count(*) || ' ' || md5(string_agg(md5(id::text || site || pad), ',' ORDER BY id)) FROM offwin;")"
ev "A: $SUM_A"
ev "B: $SUM_B"
ev "finished $(now)"

ok=1
[[ -n "$LOST_AT" ]] || { ev "FAIL: the slot was never invalidated"; ok=0; }
[[ "$LOST_REPORTED" == 1 ]] || { ev "FAIL: B's subscription read 'replicating' after its slot was lost"; ok=0; }
[[ "$st" == replicating && "$sync" == r && "$SUM_A" == "$SUM_B" ]] || { ev "FAIL: the re-seed did not converge"; ok=0; }
q b postgres "SELECT wal_status FROM pg_replication_slots WHERE slot_name LIKE '%sub_b_to_a%';" | grep -qx lost && { ev "FAIL: the copy cost A its slot on B"; ok=0; }
[[ "$ok" == 1 ]] && ev "PASS" && exit 0
exit 1
