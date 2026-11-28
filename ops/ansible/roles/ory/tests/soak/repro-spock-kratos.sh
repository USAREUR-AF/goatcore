#!/usr/bin/env bash
#
# ╔════════════════════════════════════════════════════════════════════════╗
# ║ RUNS ON: integdev (Linux EC2) — DO NOT run locally on macOS.           ║
# ║                                                                        ║
# ║ Why: F-008 (docs/findings/F-008-docker-exec-psql-hang-under-load.md). ║
# ║      The 250 ms-cadence `docker exec` polling here is exactly the      ║
# ║      pattern that wedges macOS Docker Desktop irrecoverably (full      ║
# ║      system restart required). Linux Docker handles this fine.         ║
# ║                                                                        ║
# ║ Run via: ops/ansible/roles/ory/tests/soak/run-repro-on-integdev.sh    ║
# ║          (the wrapper pushes this script over SSM, runs it on the     ║
# ║          integdev host, pulls the output dir back).                   ║
# ╚════════════════════════════════════════════════════════════════════════╝
#
# Focused reproducer for the kratos-spock-slot-dies-under-load failure.
#
# Strips everything not load-bearing: NO Hydra, NO Kratos servers, NO Ory
# probes. Only postgres-a + postgres-b + Kratos schema (via kratos-migrate)
# + bidirectional Spock subscription on the kratos DB. Then a writer loop
# at controlled rates and a polling loop at 250ms.
#
# Design: continuous observability, NOT "run + grep at end."
#  - Capture loop polls every 250ms, writes CSV + tee to stdout
#  - On slot active=t→f, immediately dumps full Spock state to disk
#  - Writer ramps 1→2→5→10→20→50 inserts/sec, 30s each
#  - On slot death OR 5 min, test ends. NO docker compose down trap —
#    crime scene stays up for follow-up queries.
#
# Output dir: /tmp/repro-spock-kratos-<ts>/
#   spock-state.csv          — 250ms-cadence slot/lag/sender state
#   writer.log               — per-insert outcome (rc + lsn delta)
#   transition-<ts>/         — full state dump on each t→f transition
#     pg_stat_activity.txt
#     spock_exception_log.txt
#     spock_exception_status.txt
#     spock_apply_worker_status.txt
#     spock_lag_tracker.txt
#     spock_channel_summary.txt
#     pg_stat_replication.txt
#     postgres-a-recent-logs.txt + postgres-b-recent-logs.txt
#
# Usage:
#   ops/ansible/roles/ory/tests/soak/repro-spock-kratos.sh

set -uo pipefail
# NB: NOT set -e. Some polling commands are expected to fail transiently.

TS="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="/tmp/repro-spock-kratos-$TS"
mkdir -p "$OUT"
echo "[repro] output directory: $OUT"

# Image tag is configurable so the script runs unchanged on Mac (local
# build of postgres-spock-test:contribs) and integdev (GHCR-pulled image
# already present from mgmt-host compose). F-008 says only run on Linux.
export POSTGRES_SPOCK_IMAGE="${POSTGRES_SPOCK_IMAGE:-postgres-spock-test:contribs}"
echo "[repro] using POSTGRES_SPOCK_IMAGE=$POSTGRES_SPOCK_IMAGE"

# Fixed test secrets — fixture only.
PASS="repro-pg-fixed-aaaa-bbbb-cccc"
SHARED_NID="00000000-0000-4000-8000-000000000001"
PG_A_PORT=26435
PG_B_PORT=26436

# Compose file generated inline so this script is self-contained.
WORK="$(mktemp -d -t repro-spock-XXXXXX)"
cat > "$WORK/docker-compose.yml" <<EOF
services:
  postgres-a:
    image: ${POSTGRES_SPOCK_IMAGE}
    # The image's 00-verify-config.sql fails fast if the baked Spock
    # postgresql.conf isn't loaded (default PGDATA config has wal_level=
    # replica, but Spock requires logical). Mirror the mgmt-host compose.
    command: ["postgres", "-c", "config_file=/etc/postgresql/postgresql.conf"]
    container_name: repro-spock-postgres-a-$TS
    environment:
      POSTGRES_PASSWORD: $PASS
      POSTGRES_USER: postgres
      POSTGRES_DB: postgres
    ports: ["$PG_A_PORT:5432"]
    networks: [reproN]
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 2s
      timeout: 2s
      retries: 30
  postgres-b:
    image: ${POSTGRES_SPOCK_IMAGE}
    # The image's 00-verify-config.sql fails fast if the baked Spock
    # postgresql.conf isn't loaded (default PGDATA config has wal_level=
    # replica, but Spock requires logical). Mirror the mgmt-host compose.
    command: ["postgres", "-c", "config_file=/etc/postgresql/postgresql.conf"]
    container_name: repro-spock-postgres-b-$TS
    environment:
      POSTGRES_PASSWORD: $PASS
      POSTGRES_USER: postgres
      POSTGRES_DB: postgres
    ports: ["$PG_B_PORT:5432"]
    networks: [reproN]
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 2s
      timeout: 2s
      retries: 30
  kratos-migrate-a:
    image: oryd/kratos:v26.2.0@sha256:2a13bb8d362c7a7ae33bd7c0f5168aee46921f15c916a06346db91c06dc76643
    container_name: repro-spock-kratos-migrate-a-$TS
    command: ["migrate", "sql", "up", "-e", "-y"]
    environment:
      DSN: "postgres://postgres:$PASS@postgres-a:5432/kratos?sslmode=disable&max_conns=2"
    depends_on:
      postgres-a: {condition: service_healthy}
    networks: [reproN]
    restart: "no"
  kratos-migrate-b:
    image: oryd/kratos:v26.2.0@sha256:2a13bb8d362c7a7ae33bd7c0f5168aee46921f15c916a06346db91c06dc76643
    container_name: repro-spock-kratos-migrate-b-$TS
    command: ["migrate", "sql", "up", "-e", "-y"]
    environment:
      DSN: "postgres://postgres:$PASS@postgres-b:5432/kratos?sslmode=disable&max_conns=2"
    depends_on:
      postgres-b: {condition: service_healthy}
    networks: [reproN]
    restart: "no"
networks:
  reproN:
    name: reproN-$TS
EOF

PG_A="repro-spock-postgres-a-$TS"
PG_B="repro-spock-postgres-b-$TS"

psqlA() { timeout 8 docker exec -e PGPASSWORD=$PASS $PG_A psql -U postgres -v ON_ERROR_STOP=1 "$@"; }
psqlB() { timeout 8 docker exec -e PGPASSWORD=$PASS $PG_B psql -U postgres -v ON_ERROR_STOP=1 "$@"; }

# F-006: postgres-spock image's init scripts trigger a post-init server
# restart that briefly fails subsequent connections. A single SELECT 1
# can succeed mid-init, then the next command hits the restart window.
# Require N consecutive successes spanning > restart-window seconds.
wait_pg() {
  local cont=$1
  local consec=0
  for _i in $(seq 1 120); do
    if timeout 5 docker exec -e PGPASSWORD=$PASS $cont psql -U postgres -d postgres -tAXc "SELECT 1;" >/dev/null 2>&1; then
      consec=$((consec+1))
      if [ $consec -ge 5 ]; then return 0; fi
    else
      consec=0
    fi
    sleep 1
  done
  return 1
}

# Bring up postgres + run kratos-migrate (creates kratos DB + schema on both).
echo "[repro] up postgres-a + postgres-b"
docker compose -f "$WORK/docker-compose.yml" up -d postgres-a postgres-b

echo "[repro] wait for postgres-a stably ready (5 consecutive SELECT 1, F-006-aware)"
wait_pg "$PG_A" || { echo "[repro] FATAL: postgres-a never became stably ready"; exit 1; }
echo "[repro] wait for postgres-b stably ready"
wait_pg "$PG_B" || { echo "[repro] FATAL: postgres-b never became stably ready"; exit 1; }

# Defensive: another small settle window. Init can spawn extension catalog
# work even after SELECT 1 returns; CREATE DATABASE during that window is
# unreliable in practice.
sleep 3

echo "[repro] create kratos DB on both nodes"
psqlA -d postgres -c "CREATE DATABASE kratos;" 2>&1 | grep -vE "^$" | head -3 || true
psqlB -d postgres -c "CREATE DATABASE kratos;" 2>&1 | grep -vE "^$" | head -3 || true

echo "[repro] verify kratos DB exists on both nodes (FAIL LOUD if missing)"
A_HAS=$(psqlA -d postgres -tAXc "SELECT 1 FROM pg_database WHERE datname='kratos';" 2>&1 | tr -d ' ')
B_HAS=$(psqlB -d postgres -tAXc "SELECT 1 FROM pg_database WHERE datname='kratos';" 2>&1 | tr -d ' ')
[ "$A_HAS" = "1" ] || { echo "[repro] FATAL: kratos DB missing on A after CREATE — got: '$A_HAS'"; exit 1; }
[ "$B_HAS" = "1" ] || { echo "[repro] FATAL: kratos DB missing on B after CREATE — got: '$B_HAS'"; exit 1; }
echo "[repro] kratos DB confirmed present on both nodes"

echo "[repro] CREATE EXTENSION spock on kratos DB on both nodes"
psqlA -d kratos -c "CREATE EXTENSION IF NOT EXISTS spock;" >/dev/null
psqlB -d kratos -c "CREATE EXTENSION IF NOT EXISTS spock;" >/dev/null

echo "[repro] run kratos-migrate against both"
docker compose -f "$WORK/docker-compose.yml" up kratos-migrate-a kratos-migrate-b
sleep 2

echo "[repro] verify identities table exists on both nodes (FAIL LOUD if missing)"
A_TBL=$(psqlA -d kratos -tAXc "SELECT 1 FROM information_schema.tables WHERE table_schema='public' AND table_name='identities';" 2>&1 | tr -d ' ')
B_TBL=$(psqlB -d kratos -tAXc "SELECT 1 FROM information_schema.tables WHERE table_schema='public' AND table_name='identities';" 2>&1 | tr -d ' ')
[ "$A_TBL" = "1" ] || { echo "[repro] FATAL: identities table missing on A after kratos-migrate — got: '$A_TBL'"; exit 1; }
[ "$B_TBL" = "1" ] || { echo "[repro] FATAL: identities table missing on B after kratos-migrate — got: '$B_TBL'"; exit 1; }
echo "[repro] identities table confirmed present on both nodes"

echo "[repro] pre-seed shared networks row on both nodes BEFORE Spock setup"
psqlA -d kratos -tAXc "INSERT INTO networks (id, created_at, updated_at) VALUES ('$SHARED_NID', NOW(), NOW()) ON CONFLICT (id) DO NOTHING;" >/dev/null
psqlB -d kratos -tAXc "INSERT INTO networks (id, created_at, updated_at) VALUES ('$SHARED_NID', NOW(), NOW()) ON CONFLICT (id) DO NOTHING;" >/dev/null

echo "[repro] configure Spock node + bidirectional subscription on kratos DB"
psqlA -d kratos -c "SELECT spock.node_create(node_name := 'node_a_kratos', dsn := 'host=postgres-a port=5432 dbname=kratos user=postgres password=$PASS');" >/dev/null
psqlB -d kratos -c "SELECT spock.node_create(node_name := 'node_b_kratos', dsn := 'host=postgres-b port=5432 dbname=kratos user=postgres password=$PASS');" >/dev/null

# Add tables to repset on both sides.
ADD_TABLES_SQL="
DO \$\$
DECLARE t text; added int := 0; skipped int := 0;
BEGIN
  FOR t IN SELECT tablename FROM pg_tables WHERE schemaname='public' LOOP
    BEGIN
      PERFORM spock.repset_add_table('default', format('%I.%I', 'public', t)::regclass);
      added := added + 1;
    EXCEPTION WHEN OTHERS THEN skipped := skipped + 1;
    END;
  END LOOP;
  RAISE NOTICE 'added=%, skipped=%', added, skipped;
END;
\$\$;"
psqlA -d kratos -c "$ADD_TABLES_SQL" 2>&1 | grep -E "(NOTICE|added)" | head -2
psqlB -d kratos -c "$ADD_TABLES_SQL" 2>&1 | grep -E "(NOTICE|added)" | head -2

DSN_A_TO_B="host=postgres-b port=5432 user=postgres password=$PASS dbname=kratos"
DSN_B_TO_A="host=postgres-a port=5432 user=postgres password=$PASS dbname=kratos"
psqlA -d kratos -c "SELECT spock.sub_create(subscription_name := 'sub_a_from_b', provider_dsn := '$DSN_A_TO_B', synchronize_data := false);" >/dev/null
psqlB -d kratos -c "SELECT spock.sub_create(subscription_name := 'sub_b_from_a', provider_dsn := '$DSN_B_TO_A', synchronize_data := false);" >/dev/null

# Wait for subscriptions to actually start replicating.
sleep 3

# ─── Capture loop (background) ───────────────────────────────────────────
# Polls every 250ms. Writes a CSV row per poll. tees to stdout.
# On slot.active transition t→f, calls dump_state.
CSV="$OUT/spock-state.csv"
echo "ts,node,slot_name,active,restart_lsn,confirmed_flush_lsn,lag_bytes,sender_count" > "$CSV"

dump_state() {
  local node=$1 slot=$2 transition_ts=$3
  local d="$OUT/transition-$transition_ts-$node-$slot"
  mkdir -p "$d"
  echo "[repro] *** SLOT TRANSITION on $node/$slot at $transition_ts — dumping state to $d ***" | tee -a "$OUT/events.log"
  local CONT
  if [ "$node" = "A" ]; then CONT=$PG_A; else CONT=$PG_B; fi
  for q_pair in \
      "pg_stat_activity.txt:SELECT pid, datname, application_name, backend_type, state, wait_event_type, wait_event, query_start, state_change, left(query, 200) AS query FROM pg_stat_activity ORDER BY backend_type, datname;" \
      "pg_stat_replication.txt:SELECT * FROM pg_stat_replication;" \
      "pg_replication_slots.txt:SELECT slot_name, active, restart_lsn, confirmed_flush_lsn, pg_wal_lsn_diff(pg_current_wal_lsn(), confirmed_flush_lsn) AS lag_bytes FROM pg_replication_slots;" \
      "spock_exception_log.txt:SELECT * FROM spock.exception_log ORDER BY 1 DESC LIMIT 50;" \
      "spock_exception_status.txt:SELECT * FROM spock.exception_status;" \
      "spock_exception_status_detail.txt:SELECT * FROM spock.exception_status_detail LIMIT 50;" \
      "spock_apply_worker_status.txt:SELECT * FROM spock.get_apply_worker_status();" \
      "spock_lag_tracker.txt:SELECT * FROM spock.lag_tracker;" \
      "spock_channel_summary_stats.txt:SELECT * FROM spock.channel_summary_stats;" \
      "spock_sub_show_status.txt:SELECT * FROM spock.sub_show_status();" \
      ; do
    fname=${q_pair%%:*}
    sql=${q_pair#*:}
    timeout 8 docker exec -e PGPASSWORD=$PASS $CONT psql -U postgres -d kratos -c "$sql" > "$d/$fname" 2>&1
  done
  # Postgres logs (last 200 lines from each)
  docker logs --tail 200 $PG_A > "$d/postgres-a-recent.log" 2>&1
  docker logs --tail 200 $PG_B > "$d/postgres-b-recent.log" 2>&1
  echo "[repro] dump complete: $d" | tee -a "$OUT/events.log"
}

(
  # Track previous slot.active state per (node,slot)
  declare -A prev_active
  while :; do
    ts=$(date -u +%Y-%m-%dT%H:%M:%S.%3NZ)
    # Sender count per node
    sa=$(timeout 5 docker exec -e PGPASSWORD=$PASS $PG_A psql -U postgres -d kratos -tAXc "SELECT count(*) FROM pg_stat_replication;" 2>/dev/null | tr -d ' ')
    sb=$(timeout 5 docker exec -e PGPASSWORD=$PASS $PG_B psql -U postgres -d kratos -tAXc "SELECT count(*) FROM pg_stat_replication;" 2>/dev/null | tr -d ' ')
    [ -z "$sa" ] && sa=0
    [ -z "$sb" ] && sb=0
    # Slot snapshot per node
    while IFS='|' read -r slot active rlsn flsn lag; do
      [ -z "$slot" ] && continue
      slot=$(echo $slot | tr -d ' '); active=$(echo $active | tr -d ' '); lag=$(echo $lag | tr -d ' ')
      key="A_$slot"
      prev=${prev_active[$key]:-}
      if [ "$prev" = "t" ] && [ "$active" = "f" ]; then dump_state "A" "$slot" "$ts"; fi
      prev_active[$key]=$active
      echo "$ts,A,$slot,$active,$rlsn,$flsn,$lag,$sa" >> "$CSV"
    done < <(timeout 5 docker exec -e PGPASSWORD=$PASS $PG_A psql -U postgres -d kratos -tAXc "SELECT slot_name||'|'||active||'|'||restart_lsn||'|'||confirmed_flush_lsn||'|'||pg_wal_lsn_diff(pg_current_wal_lsn(), confirmed_flush_lsn) FROM pg_replication_slots WHERE plugin='spock_output';" 2>/dev/null)
    while IFS='|' read -r slot active rlsn flsn lag; do
      [ -z "$slot" ] && continue
      slot=$(echo $slot | tr -d ' '); active=$(echo $active | tr -d ' '); lag=$(echo $lag | tr -d ' ')
      key="B_$slot"
      prev=${prev_active[$key]:-}
      if [ "$prev" = "t" ] && [ "$active" = "f" ]; then dump_state "B" "$slot" "$ts"; fi
      prev_active[$key]=$active
      echo "$ts,B,$slot,$active,$rlsn,$flsn,$lag,$sb" >> "$CSV"
    done < <(timeout 5 docker exec -e PGPASSWORD=$PASS $PG_B psql -U postgres -d kratos -tAXc "SELECT slot_name||'|'||active||'|'||restart_lsn||'|'||confirmed_flush_lsn||'|'||pg_wal_lsn_diff(pg_current_wal_lsn(), confirmed_flush_lsn) FROM pg_replication_slots WHERE plugin='spock_output';" 2>/dev/null)
    sleep 0.25
  done
) &
CAPTURE_PID=$!
echo "[repro] capture loop PID $CAPTURE_PID writing $CSV"

# ─── Periodic terminal status (so user can watch) ─────────────────────────
(
  while :; do
    sleep 5
    # Show latest CSV row per slot
    awk -F, 'NR>1 {key=$2"_"$3; row[key]=$0} END {for (k in row) print row[k]}' "$CSV" 2>/dev/null \
      | sort | sed 's/^/[status] /'
  done
) &
STATUS_PID=$!

# ─── Writer loop (background) ─────────────────────────────────────────────
# Inserts identities at A at controlled rates: 1, 2, 5, 10, 20, 50 inserts/sec
# 30s per rate. Stops on slot death OR 5 min total.
WRITER_LOG="$OUT/writer.log"
echo "ts,rate,iter,rc,duration_ms" > "$WRITER_LOG"
(
  iter=0
  for rate in 1 2 5 10 20 50; do
    period_ms=$((1000 / rate))
    end_at=$((SECONDS + 30))
    echo "[writer] rate=$rate/s for 30s (period=${period_ms}ms)" | tee -a "$OUT/events.log"
    while [ $SECONDS -lt $end_at ]; do
      iter=$((iter+1))
      uuid=$(uuidgen | tr A-Z a-z)
      email="repro-$iter@repro.local"
      sql="INSERT INTO identities (id, nid, schema_id, traits, state, state_changed_at, created_at, updated_at) VALUES ('$uuid', '$SHARED_NID', 'default', '{\"email\":\"$email\"}', 'active', NOW(), NOW(), NOW());"
      t0=$(date +%s%3N)
      timeout 5 docker exec -e PGPASSWORD=$PASS $PG_A psql -U postgres -d kratos -tAXc "$sql" >/dev/null 2>&1
      rc=$?
      t1=$(date +%s%3N)
      ts=$(date -u +%Y-%m-%dT%H:%M:%S.%3NZ)
      echo "$ts,$rate,$iter,$rc,$((t1-t0))" >> "$WRITER_LOG"
      # pace to target rate (best-effort; if exec is slow, we just go slower)
      target_done=$((t0 + period_ms))
      now=$(date +%s%3N)
      sleep_ms=$((target_done - now))
      if [ "$sleep_ms" -gt 0 ]; then sleep "$(awk "BEGIN{print $sleep_ms/1000}")"; fi
    done
  done
  echo "[writer] all rates exhausted at $iter inserts" | tee -a "$OUT/events.log"
) &
WRITER_PID=$!
echo "[repro] writer loop PID $WRITER_PID at rates 1→50/s, log $WRITER_LOG"

# ─── Termination logic ────────────────────────────────────────────────────
# End when: (a) writer finishes, OR (b) any slot dies AND stays dead for 30s,
# OR (c) 5 min wall clock.
WALL_END=$((SECONDS + 300))
SLOT_DEAD_AT=
while :; do
  if ! kill -0 $WRITER_PID 2>/dev/null; then
    echo "[repro] writer exited" | tee -a "$OUT/events.log"
    break
  fi
  if [ $SECONDS -ge $WALL_END ]; then
    echo "[repro] 5 min wall clock reached" | tee -a "$OUT/events.log"
    break
  fi
  # Quick check: any slot.active=f?
  any_dead=$(tail -50 "$CSV" 2>/dev/null | awk -F, '$4=="f"{print "1"; exit}')
  if [ "$any_dead" = "1" ]; then
    if [ -z "$SLOT_DEAD_AT" ]; then
      SLOT_DEAD_AT=$SECONDS
      echo "[repro] slot died — letting writer continue 30s for context, then stopping" | tee -a "$OUT/events.log"
    elif [ $((SECONDS - SLOT_DEAD_AT)) -ge 30 ]; then
      echo "[repro] 30s post-death window elapsed" | tee -a "$OUT/events.log"
      break
    fi
  fi
  sleep 2
done

echo "[repro] stopping writer + capture + status loops"
kill $WRITER_PID 2>/dev/null
kill $CAPTURE_PID 2>/dev/null
kill $STATUS_PID 2>/dev/null
sleep 1

# Final summary
echo "[repro] === SUMMARY ==="
echo "  output dir: $OUT"
echo "  csv rows:   $(wc -l < "$CSV")"
echo "  writes:     $(($(wc -l < "$WRITER_LOG") - 1))"
transition_count=0
for transition in "$OUT"/transition-*; do
  if [ -e "$transition" ] || [ -L "$transition" ]; then
    transition_count=$((transition_count + 1))
  fi
done
echo "  transitions captured: $transition_count"
echo "  events.log:"
cat "$OUT/events.log" 2>/dev/null | sed 's/^/    /'

echo "[repro] CRIME SCENE PRESERVED. Containers $PG_A and $PG_B still running."
echo "[repro] Manual teardown when done:"
echo "  docker compose -f $WORK/docker-compose.yml down -v"
echo "  rm -rf $WORK"
