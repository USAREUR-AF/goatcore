#!/usr/bin/env bash
# Functional test of ops/ansible/postgres-encoding-migrate.yml on one Docker
# host: a founder and a joiner, each a compose project with the goatnet's
# postgres-spock image, SQL_ASCII clusters wired with the real
# spock-mesh-peering role (spock-mesh-wire.yml), a session-holding writer in
# each project, a writer outside any project, a container that mounts PGDATA,
# and a pgBackRest stanza on the founder.
#
# Cases, in order:
#   0  rows written on each node before Spock was wired (synchronize_data
#      false) leave a replicated parent table different and a replicated child
#      row orphaned: phase 1 refuses, accept_divergence does not get past it,
#      reconcile refuses a conflicting key, then copies the missing rows without
#      echoing them through Spock
#   1  a value that is not UTF-8 stops phase 1 and names its table
#   2  migrate with stop_after=4, then resume to the end
#   3  result: every database UTF8, rows and bytes equal, owners and settings
#      kept, subscriptions replicating, a new write replicates both ways,
#      writers back, pgBackRest has a backup of the new cluster
#   4  a re-run is a no-op
#   5  rollback: SQL_ASCII again, old data, old subscriptions replicating
#      The founder's pgBackRest config matches the role's (archive-async, spool,
#      archive_timeout), so an archive-push fires on its own while the new
#      cluster has no stanza yet.
#   6  migrate again with repair=latin1 over a LATIN1 value (a new run id;
#      the rolled-back run's copy is reported), then cleanup of both runs
#
# Needs: docker (compose v2), ansible-playbook, the postgres-spock image
# (POSTGRES_SPOCK_IMAGE, else the fleet pin). KEEP=1 leaves everything up.

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ANS="$(cd "$HERE/../../.." && pwd)"           # ops/ansible
PLAY="$ANS/postgres-encoding-migrate.yml"
WORK="${WORK:-$(mktemp -d /tmp/pem-test.XXXXXX)}"
AP="${ANSIBLE_PLAYBOOK:-ansible-playbook}"
PW=pemtest-superuser
export ANSIBLE_CONFIG="$ANS/ansible.cfg"
export UTF8_FOUNDER_REPLICATOR_PASS=pemtest-replicator
export UTF8_JOINER_REPLICATOR_PASS=pemtest-replicator
NODES=(founder joiner)

say() { printf '\n[pem-test] %s\n' "$*"; }
fail() { printf '\n[pem-test] FAIL: %s\n' "$*" >&2; exit 1; }

if [[ -z "${POSTGRES_SPOCK_IMAGE:-}" ]]; then
  POSTGRES_SPOCK_IMAGE="$(sed -n '/^  postgres-spock:/,/^  [a-z]/s/^ *fleet: //p' "$ANS/../pins/fleet-pins.yaml" | head -1 \
    | sed "s/{{ *ghcr_org *}}/${GHCR_ORG:-usareur-af}/")"
fi
[[ -n "$POSTGRES_SPOCK_IMAGE" ]] || fail "set POSTGRES_SPOCK_IMAGE"
export POSTGRES_SPOCK_IMAGE
say "image $POSTGRES_SPOCK_IMAGE, work dir $WORK"

dc() { docker compose -p "pemtest-$1" --project-directory "$WORK/$1" -f "$WORK/$1/docker-compose.yml" "${@:2}"; }
pg() { local n=$1 db=$2; shift 2; docker exec -i -e PGCLIENTENCODING=UTF8 -u postgres "pemtest-$n-pg" psql -X -q -v ON_ERROR_STOP=1 -U postgres -tA -d "$db" "$@"; }
asroot() { docker run --rm --user 0 --entrypoint sh -v "$1":/p "$POSTGRES_SPOCK_IMAGE" -c "$2"; }

teardown() {
  for n in "${NODES[@]}"; do
    [[ -f "$WORK/$n/docker-compose.yml" ]] && dc "$n" down -v --remove-orphans >/dev/null 2>&1 || true
    docker rm -f "pemtest-$n-extwriter" >/dev/null 2>&1 || true
    docker volume ls -q --filter label=goat.postgres-utf8-migration.of="pemtest-${n}_pgdata-a" | xargs -r docker volume rm -f >/dev/null 2>&1 || true
  done
  docker network rm pemtest-net >/dev/null 2>&1 || true
  [[ -d "$WORK" ]] && asroot "$WORK" 'rm -rf /p/founder /p/joiner /p/state-founder /p/state-joiner' >/dev/null 2>&1 || true
}
cleanup() {
  if [[ "${KEEP:-0}" == 1 ]]; then say "KEEP=1: left running in $WORK"; return; fi
  teardown
}
trap cleanup EXIT
teardown

wait_pg() {
  local n=$1 ok=0
  for _ in $(seq 1 90); do
    if pg "$n" postgres -c 'SELECT 1' >/dev/null 2>&1; then ok=$((ok + 1)); [[ $ok -ge 3 ]] && return 0; else ok=0; fi
    sleep 2
  done
  fail "pemtest-$n-pg never became ready"
}

run_play() {   # run_play <log> <extra args...>
  local log=$1; shift
  ( cd "$ANS" && "$AP" -i "$WORK/inventory.ini" "$PLAY" -e migrate_groups=utf8_founder_hosts,utf8_joiner_hosts "$@" ) > "$log" 2>&1
}

# ---------------------------------------------------------------- fixture
say "fixture: network, two SQL_ASCII compose projects"
docker network create pemtest-net >/dev/null
for n in "${NODES[@]}"; do
  mkdir -p "$WORK/$n"/{pgbackrest-repo,pgbackrest-etc,pgbackrest-log,pgbackrest-spool} "$WORK/state-$n"
  sed "s/@NODE@/$n/g" "$HERE/compose.yml.tmpl" > "$WORK/$n/docker-compose.yml"
  printf 'POSTGRES_SUPERUSER_PASSWORD=%s\nPOSTGRES_SPOCK_IMAGE=%s\nPOSTGRES_INITDB_ARGS="--locale=C --encoding=SQL_ASCII"\n' \
    "$PW" "$POSTGRES_SPOCK_IMAGE" > "$WORK/$n/.env"
  asroot "$WORK/$n" 'chown 26:26 /p/pgbackrest-repo /p/pgbackrest-log /p/pgbackrest-spool && chmod 750 /p/pgbackrest-repo'
done
cat > "$WORK/founder/pgbackrest-etc/pgbackrest.conf" <<'EOF'
[global]
repo1-path=/var/lib/pgbackrest
repo1-cipher-type=aes-256-cbc
repo1-cipher-pass=pemtest-cipher
repo1-retention-full=2
log-path=/var/log/pgbackrest
spool-path=/var/spool/pgbackrest
archive-async=y
compress-type=zst
log-level-console=info
log-level-file=detail

[utf8-founder-netbird]
pg1-path=/var/lib/pgsql/16/data
pg1-socket-path=/var/run/postgresql
pg1-user=postgres
EOF
chmod 644 "$WORK/founder/pgbackrest-etc/pgbackrest.conf"

cat > "$WORK/inventory.ini" <<EOF
[utf8_founder_hosts]
pem-founder ansible_host=127.0.0.1 site_name=utf8-founder signal_uri=pemtest-founder-pg:5432 spock_mesh_postgres_container=pemtest-founder-pg postgres_encoding_migrate_dir=$WORK/state-founder postgres_encoding_migrate_pgbackrest_conf=$WORK/founder/pgbackrest-etc/pgbackrest.conf postgres_encoding_migrate_pgbackrest_repo=$WORK/founder/pgbackrest-repo postgres_encoding_migrate_pgbackrest_spool=$WORK/founder/pgbackrest-spool ansible_python_interpreter=$(command -v python3)

[utf8_joiner_hosts]
pem-joiner ansible_host=127.0.0.1 site_name=utf8-joiner signal_uri=pemtest-joiner-pg:5432 spock_mesh_postgres_container=pemtest-joiner-pg postgres_encoding_migrate_dir=$WORK/state-joiner postgres_encoding_migrate_pgbackrest_conf=$WORK/joiner/pgbackrest-etc/pgbackrest.conf postgres_encoding_migrate_pgbackrest_repo=$WORK/joiner/pgbackrest-repo postgres_encoding_migrate_pgbackrest_spool=$WORK/joiner/pgbackrest-spool ansible_python_interpreter=$(command -v python3)

[mgmt_hosts:children]
utf8_founder_hosts
utf8_joiner_hosts
EOF
rm -rf "$WORK/group_vars"; cp -r "$HERE/group_vars" "$WORK/group_vars"

for n in "${NODES[@]}"; do dc "$n" up -d postgres-a >/dev/null; done
for n in "${NODES[@]}"; do wait_pg "$n"; done
for n in "${NODES[@]}"; do
  [[ "$(pg "$n" postgres -c "SELECT pg_encoding_to_char(encoding) FROM pg_database WHERE datname='netbird'")" == SQL_ASCII ]] \
    || fail "$n did not initialise SQL_ASCII"
done

say "fixture: schema and data (UTF-8 text stored in SQL_ASCII)"
for n in "${NODES[@]}"; do
  pg "$n" netbird -c "CREATE TABLE utf8_shared (id text PRIMARY KEY, v text NOT NULL);
                      CREATE TABLE utf8_local (id serial PRIMARY KEY, v text NOT NULL);
                      INSERT INTO utf8_local (v) SELECT '$n local ' || g || ' — Ærø' FROM generate_series(1, 50) g;"
  pg "$n" netbird_events -c "CREATE TABLE events (id bigserial PRIMARY KEY, at timestamptz DEFAULT now(), msg text);
                             INSERT INTO events (msg) SELECT '$n event ' || g FROM generate_series(1, 200) g;"
  pg "$n" postgres -c "CREATE ROLE pemtest_app LOGIN PASSWORD 'pemtest-app';"
  pg "$n" postgres -c "CREATE DATABASE pemtest_app OWNER pemtest_app;"
  pg "$n" postgres -c "ALTER DATABASE pemtest_app SET search_path = app, public; GRANT CONNECT ON DATABASE pemtest_app TO PUBLIC;"
  pg "$n" pemtest_app -c "CREATE SCHEMA app AUTHORIZATION pemtest_app; CREATE TABLE app.notes (id int PRIMARY KEY, body text);
                          ALTER TABLE app.notes OWNER TO pemtest_app; INSERT INTO app.notes VALUES (1, 'Ελληνικά'), (2, '東京 ✓');"
done
pg founder postgres -c "ALTER SYSTEM SET archive_mode = 'on';"
pg founder postgres -c "ALTER SYSTEM SET archive_command = 'pgbackrest --stanza=utf8-founder-netbird archive-push %p';"
# As the pgbackrest role sets it (60s there): a WAL switch, and so an
# archive-push, happens on an idle cluster too.
pg founder postgres -c "ALTER SYSTEM SET archive_timeout = '10s';"
# A checkpoint soon after any start, so WAL the restore wrote is marked ready
# for archiving within the phase that started it (on a mgmt host the larger
# restore does this through max_wal_size).
pg founder postgres -c "ALTER SYSTEM SET checkpoint_timeout = '30s';"
docker restart pemtest-founder-pg >/dev/null; wait_pg founder
docker exec -u postgres pemtest-founder-pg pgbackrest --stanza=utf8-founder-netbird stanza-create >/dev/null
docker exec -u postgres pemtest-founder-pg pgbackrest --stanza=utf8-founder-netbird backup --type=full >/dev/null

say "fixture: a parent row of each node's own, written before Spock is wired"
for n in "${NODES[@]}"; do
  pg "$n" netbird -c "CREATE TABLE utf8_net (nid text PRIMARY KEY, note text NOT NULL);
                      CREATE TABLE utf8_client (id text PRIMARY KEY, nid text NOT NULL REFERENCES utf8_net (nid));
                      INSERT INTO utf8_net VALUES ('shared', 'the same on every node'), ('nid-$n', '$n network');"
done

say "fixture: Spock wiring with the real role (founder, then joiner)"
for h in pem-founder pem-joiner; do
  ( cd "$ANS" && "$AP" -i "$WORK/inventory.ini" spock-mesh-wire.yml -l "$h" ) > "$WORK/wire-$h.log" 2>&1 \
    || { tail -40 "$WORK/wire-$h.log"; fail "spock-mesh-wire on $h"; }
done
pg founder netbird -c "INSERT INTO utf8_shared VALUES ('f1', 'café'), ('f2', 'Zürich'), ('f3', '東京'), ('f4', 'emoji 🐐');"
pg joiner netbird -c "INSERT INTO utf8_shared VALUES ('j1', 'ñandú'), ('j2', 'Ελλάδα');"
for i in $(seq 1 30); do
  a=$(pg founder netbird -c "SELECT count(*) FROM utf8_shared"); b=$(pg joiner netbird -c "SELECT count(*) FROM utf8_shared")
  [[ "$a" == 6 && "$b" == 6 ]] && break; sleep 1
  [[ $i == 30 ]] && fail "fixture replication: founder $a joiner $b rows"
done

say "fixture: a child row on each node referencing its own parent, replicated"
pg founder netbird -c "INSERT INTO utf8_client VALUES ('c-founder', 'nid-founder');"
pg joiner netbird -c "INSERT INTO utf8_client VALUES ('c-joiner', 'nid-joiner');"
for i in $(seq 1 30); do
  a=$(pg founder netbird -c "SELECT count(*) FROM utf8_client"); b=$(pg joiner netbird -c "SELECT count(*) FROM utf8_client")
  [[ "$a" == 2 && "$b" == 2 ]] && break; sleep 1
  [[ $i == 30 ]] && fail "fixture: child rows did not replicate (founder $a joiner $b)"
done
[[ "$(pg joiner netbird -c "SELECT count(*) FROM utf8_client c WHERE NOT EXISTS (SELECT 1 FROM utf8_net p WHERE p.nid = c.nid)")" == 1 ]] \
  || fail "fixture: the joiner should hold one orphan child row"

say "fixture: writers (in the project, and one outside any project)"
for n in "${NODES[@]}"; do
  dc "$n" up -d writer volreader >/dev/null
  docker run -d --name "pemtest-$n-extwriter" --network pemtest-net -e PGPASSWORD="$PW" --entrypoint sh "$POSTGRES_SPOCK_IMAGE" \
    -c "while true; do psql -h pemtest-$n-pg -U postgres -d netbird_events -c 'SELECT pg_sleep(86400)' >/dev/null 2>&1; sleep 2; done" >/dev/null
done
sleep 4
shared_before=$(pg founder netbird -c "SELECT string_agg(id || '=' || encode(convert_to(v, 'SQL_ASCII'), 'hex'), ',' ORDER BY id) FROM utf8_shared")

# ---------------------------------------------------------------- case 1
say "case 0: divergent parent + orphan child: phase 1 refuses; reconcile fixes it"
if run_play "$WORK/case0a.log"; then fail "case 0: the run passed over an orphan"; fi
grep -q 'utf8_client -> utf8_net (utf8_client_nid_fkey): 1 row(s) without a parent' "$WORK/case0a.log" \
  || { grep -E 'msg|FAILED' "$WORK/case0a.log" | tail -5; fail "case 0: the refusal does not name the orphan"; }
grep -q 'postgres_encoding_migrate_reconcile_tables=netbird.public.utf8_net' "$WORK/case0a.log" || fail "case 0: the refusal does not name the reconcile"
[[ ! -f "$WORK/state-founder/current-run" ]] || fail "case 0: a run was opened"
if run_play "$WORK/case0b.log" -e postgres_encoding_migrate_accept_divergence=true; then fail "case 0: accept_divergence passed an orphan"; fi
grep -q 'does not change that' "$WORK/case0b.log" || fail "case 0: accept_divergence refusal does not say why"
exc_before=$(pg founder netbird -c "SELECT count(*) FROM spock.exception_log")$(pg joiner netbird -c "SELECT count(*) FROM spock.exception_log")
# A key with different content on the two nodes is a conflict, not a missing row.
pg founder netbird -c "BEGIN; SELECT spock.repair_mode(true); UPDATE utf8_net SET note = 'changed here only' WHERE nid = 'shared'; SELECT spock.repair_mode(false); COMMIT;" >/dev/null
if run_play "$WORK/case0c.log" -e postgres_encoding_migrate_action=reconcile -e postgres_encoding_migrate_reconcile_tables=netbird.public.utf8_net; then
  fail "case 0: reconcile copied over a conflicting key"; fi
grep -q 'differs between members' "$WORK/case0c.log" || fail "case 0: reconcile refusal does not name the conflict"
[[ "$(pg joiner netbird -c "SELECT count(*) FROM utf8_net")" == 2 ]] || fail "case 0: a refused reconcile wrote rows"
pg founder netbird -c "BEGIN; SELECT spock.repair_mode(true); UPDATE utf8_net SET note = 'the same on every node' WHERE nid = 'shared'; SELECT spock.repair_mode(false); COMMIT;" >/dev/null
run_play "$WORK/case0d.log" -e postgres_encoding_migrate_action=reconcile -e postgres_encoding_migrate_reconcile_tables=netbird.public.utf8_net \
  || { tail -40 "$WORK/case0d.log"; fail "case 0: reconcile"; }
sleep 3
for n in "${NODES[@]}"; do
  [[ "$(pg "$n" netbird -c "SELECT string_agg(nid, ',' ORDER BY nid) FROM utf8_net")" == 'nid-founder,nid-joiner,shared' ]] || fail "case 0: $n utf8_net after reconcile"
done
[[ "$(pg founder netbird -c "SELECT count(*) FROM spock.exception_log")$(pg joiner netbird -c "SELECT count(*) FROM spock.exception_log")" == "$exc_before" ]] \
  || fail "case 0: the copied rows echoed through Spock (exception_log grew)"
grep -q 'Named tables still different: none' "$WORK/case0d.log" || fail "case 0: reconcile did not report the table equal"
grep -q 'Foreign-key orphans: none' "$WORK/case0d.log" || fail "case 0: reconcile did not report the orphans gone"
echo "  refused with names (accept_divergence too), conflict refused, missing rows copied without echo, orphans gone"

say "case 1: a value that is not UTF-8 stops phase 1 and names its table"
pg founder netbird -c "INSERT INTO utf8_local (v) VALUES (convert_from('\\x6574e9'::bytea, 'SQL_ASCII'));"
if run_play "$WORK/case1.log"; then fail "case 1: the run passed over an invalid byte"; fi
grep -q 'public.utf8_local: 1 row' "$WORK/case1.log" || { tail -30 "$WORK/case1.log"; fail "case 1: the refusal does not name public.utf8_local"; }
[[ ! -f "$WORK/state-founder/current-run" ]] || fail "case 1: a run was opened"
docker ps --format '{{.Names}}' | grep -q pemtest-founder-writer || fail "case 1: preflight stopped a writer"
pg founder netbird -c "DELETE FROM utf8_local WHERE v = convert_from('\\x6574e9'::bytea, 'SQL_ASCII');"
echo "  refused, table named, nothing stopped"

# ---------------------------------------------------------------- case 2
say "case 2: migrate to phase 4, then resume"
seq_founder=$(pg founder netbird -c "SELECT last_value FROM utf8_local_id_seq")
seq_joiner=$(pg joiner netbird -c "SELECT last_value FROM utf8_local_id_seq")
t0=$(date +%s)
run_play "$WORK/case2a.log" -e postgres_encoding_migrate_stop_after=4 || { tail -60 "$WORK/case2a.log"; fail "case 2: phases 1-4"; }
RUN=$(cat "$WORK/state-founder/current-run")
[[ "$RUN" == "$(cat "$WORK/state-joiner/current-run")" ]] || fail "case 2: members in different runs"
for n in "${NODES[@]}"; do
  for p in 1 2 3 4; do [[ -f "$WORK/state-$n/$RUN/phase-$p.done" ]] || fail "case 2: $n phase $p not done"; done
  [[ ! -f "$WORK/state-$n/$RUN/phase-5.done" ]] || fail "case 2: $n ran past stop_after"
  [[ "$(docker inspect -f '{{.State.Running}}' "pemtest-$n-pg")" == false ]] || fail "case 2: $n Postgres still running after snapshot"
  docker volume inspect "pemtest-${n}_pgdata-a-pre-utf8-$RUN" >/dev/null || fail "case 2: $n has no kept PGDATA"
done
grep -q 'pemtest-founder-extwriter' "$WORK/state-founder/$RUN/stopped.json" || fail "case 2: the connected writer outside the project was not found"
# WAL keeps completing while the play runs, as it does on a mgmt host: a
# non-transactional message (no table touched, counts unchanged) and a WAL
# switch every 2 s on the founder. Each completed segment is handed to the
# archive command as soon as archive_mode is on.
( while :; do
    docker exec -u postgres pemtest-founder-pg psql -X -tA -d postgres \
      -c "SELECT pg_logical_emit_message(false, 'pemtest', 'tick')" -c "SELECT pg_switch_wal()" >/dev/null 2>&1 || true
    sleep 2
  done ) &
TICKER=$!
run_play "$WORK/case2b.log" || { kill $TICKER; tail -80 "$WORK/case2b.log"; docker logs pemtest-founder-pg 2>&1 | grep -E 'server process \(PID|unable to find a valid repository' | head -5; fail "case 2: resume to the end"; }
kill $TICKER 2>/dev/null || true
t1=$(date +%s)
echo "  run $RUN complete; phases 1-9 took $((t1 - t0)) s including the pause"

# ---------------------------------------------------------------- case 3
say "case 3: the result"
for n in "${NODES[@]}"; do
  bad=$(pg "$n" postgres -c "SELECT string_agg(datname, ',') FROM pg_database WHERE pg_encoding_to_char(encoding) <> 'UTF8'")
  [[ -z "$bad" ]] || fail "case 3: $n databases not UTF8: $bad"
  grep -q '^POSTGRES_INITDB_ARGS="--locale=C --encoding=UTF8"$' "$WORK/$n/.env" || fail "case 3: $n .env does not carry UTF8"
  [[ "$(pg "$n" netbird -c "SELECT count(*) FROM utf8_local")" == 50 ]] || fail "case 3: $n utf8_local rows"
  [[ "$(pg "$n" netbird_events -c "SELECT count(*) FROM events")" == 200 ]] || fail "case 3: $n events rows"
  [[ "$(pg "$n" netbird -c "SELECT v FROM utf8_local WHERE id = 1")" == "$n local 1 — Ærø" ]] || fail "case 3: $n per-site text changed"
  [[ "$(pg "$n" postgres -c "SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname='pemtest_app'")" == pemtest_app ]] || fail "case 3: $n database owner lost"
  pg "$n" postgres -c "SELECT setconfig FROM pg_db_role_setting s JOIN pg_database d ON d.oid = s.setdatabase WHERE datname='pemtest_app'" | grep -q 'search_path=app, public' \
    || fail "case 3: $n database setting lost"
  [[ "$(pg "$n" pemtest_app -c "SELECT body FROM notes WHERE id = 2")" == '東京 ✓' ]] || fail "case 3: $n app text"
  [[ "$(pg "$n" netbird -c "SELECT nextval('utf8_local_id_seq')")" == $(( $(eval echo "\$seq_$n") + 1 )) ]] || fail "case 3: $n sequence did not carry over"
  docker exec -e PGPASSWORD=pemtest-app "pemtest-$n-pg" psql -X -h 127.0.0.1 -U pemtest_app -d pemtest_app -tAc 'SELECT 1' >/dev/null \
    || fail "case 3: $n role password lost"
  for c in writer volreader extwriter; do
    [[ "$(docker inspect -f '{{.State.Running}}' "pemtest-$n-$c")" == true ]] || fail "case 3: $n $c not running again"
  done
  [[ "$(pg "$n" netbird -c "SELECT string_agg(id || '=' || encode(convert_to(v, 'UTF8'), 'hex'), ',' ORDER BY id) FROM utf8_shared")" == "$shared_before" ]] \
    || fail "case 3: $n replicated text bytes changed"
  [[ "$(pg "$n" netbird -c "SELECT count(*) FROM spock.sub_show_status() WHERE status = 'replicating'")" == 1 ]] || fail "case 3: $n subscription not replicating"
done
pg founder netbird -c "INSERT INTO utf8_shared VALUES ('f5', 'après la migration ✓');"
pg joiner netbird -c "INSERT INTO utf8_shared VALUES ('j3', 'nach der Migration ✓');"
for i in $(seq 1 30); do
  a=$(pg joiner netbird -c "SELECT v FROM utf8_shared WHERE id = 'f5'"); b=$(pg founder netbird -c "SELECT v FROM utf8_shared WHERE id = 'j3'")
  [[ "$a" == 'après la migration ✓' && "$b" == 'nach der Migration ✓' ]] && break; sleep 1
  [[ $i == 30 ]] && fail "case 3: new writes did not replicate (joiner sees '$a', founder sees '$b')"
done
docker exec -u postgres pemtest-founder-pg pgbackrest --stanza=utf8-founder-netbird info | grep -q 'full backup' || fail "case 3: no backup of the new cluster"
# The new cluster never crash-looped: archiving was off until its stanza existed.
if docker logs pemtest-founder-pg 2>&1 | grep -E 'server process \(PID [0-9]+\) exited|terminating any other active server processes|reinitializing'; then
  fail "case 3: the new cluster's Postgres crashed (above)"; fi
docker logs pemtest-founder-pg 2>&1 | grep -q 'unable to find a valid repository' && fail "case 3: archive-push ran before the new stanza existed"
[[ "$(pg founder postgres -c 'SHOW archive_mode')" == on ]] || fail "case 3: archiving is not back on"
[[ "$(pg joiner postgres -c 'SHOW archive_mode')" == off ]] || fail "case 3: the joiner (no stanza, archiving never on) changed"
docker exec -u postgres pemtest-founder-pg pgbackrest --stanza=utf8-founder-netbird check >/dev/null || fail "case 3: pgbackrest check on the new cluster"
ls -d "$WORK/founder/pgbackrest-repo.pre-utf8-$RUN" >/dev/null || fail "case 3: old pgBackRest repository not kept"
echo "  UTF8 everywhere; counts, bytes, owners, settings, sequences, passwords kept; replication both ways; writers back; no crash; archiving back on; new stanza checked and backed up"

# ---------------------------------------------------------------- case 4
say "case 4: a re-run is a no-op"
run_play "$WORK/case4.log" || { tail -40 "$WORK/case4.log"; fail "case 4"; }
if grep -E 'changed=[1-9]' "$WORK/case4.log"; then fail "case 4: the re-run changed something"; fi
echo "  changed=0"

# ---------------------------------------------------------------- case 5
say "case 5: rollback"
if run_play "$WORK/case5a.log" -e postgres_encoding_migrate_action=rollback; then fail "case 5: rollback ran without confirm"; fi
run_play "$WORK/case5.log" -e postgres_encoding_migrate_action=rollback -e postgres_encoding_migrate_confirm="$RUN" \
  || { tail -60 "$WORK/case5.log"; fail "case 5: rollback"; }
for n in "${NODES[@]}"; do
  [[ "$(pg "$n" postgres -c "SELECT pg_encoding_to_char(encoding) FROM pg_database WHERE datname='netbird'")" == SQL_ASCII ]] || fail "case 5: $n not SQL_ASCII"
  [[ "$(pg "$n" netbird -c "SELECT count(*) FROM utf8_shared")" == 6 ]] || fail "case 5: $n does not hold the pre-migration rows"
  [[ "$(pg "$n" netbird -c "SELECT count(*) FROM spock.sub_show_status() WHERE status = 'replicating'")" == 1 ]] || fail "case 5: $n old subscription not replicating"
  [[ ! -f "$WORK/state-$n/current-run" ]] || fail "case 5: $n run still open"
  [[ "$(docker inspect -f '{{.State.Running}}' "pemtest-$n-writer")" == true ]] || fail "case 5: $n writer not running"
done
pg founder netbird -c "INSERT INTO utf8_shared VALUES ('f6', 'after rollback');"
for i in $(seq 1 30); do
  [[ "$(pg joiner netbird -c "SELECT count(*) FROM utf8_shared WHERE id = 'f6'")" == 1 ]] && break; sleep 1
  [[ $i == 30 ]] && fail "case 5: the old subscription does not carry new writes"
done
docker exec -u postgres pemtest-founder-pg pgbackrest --stanza=utf8-founder-netbird check >/dev/null || fail "case 5: old pgBackRest repository not back"
echo "  SQL_ASCII again, pre-migration rows, old subscriptions replicate, old repository back"

# ---------------------------------------------------------------- case 6
say "case 6: migrate again over a LATIN1 value with repair=latin1, then cleanup"
pg founder netbird -c "INSERT INTO utf8_local (v) VALUES (convert_from('\\x6574e9'::bytea, 'SQL_ASCII'));"
latin_id=$(pg founder netbird -c "SELECT max(id) FROM utf8_local")
run_play "$WORK/case6.log" -e postgres_encoding_migrate_repair=latin1 || { tail -60 "$WORK/case6.log"; fail "case 6: migrate"; }
RUN2=$(cat "$WORK/state-founder/current-run")
[[ "$RUN2" != "$RUN" ]] || fail "case 6: the run id was reused"
[[ "$(pg founder netbird -c "SELECT v FROM utf8_local WHERE id = $latin_id")" == 'eté' ]] || fail "case 6: the LATIN1 byte was not repaired"
[[ "$(pg founder netbird -c "SELECT v FROM utf8_local WHERE id = 1")" == 'founder local 1 — Ærø' ]] || fail "case 6: repair touched valid UTF-8"
if run_play "$WORK/case6b.log" -e postgres_encoding_migrate_action=cleanup -e postgres_encoding_migrate_confirm=wrong; then fail "case 6: cleanup without the right confirm"; fi
run_play "$WORK/case6c.log" -e postgres_encoding_migrate_action=cleanup -e postgres_encoding_migrate_confirm="$RUN2" \
  || { tail -40 "$WORK/case6c.log"; fail "case 6: cleanup"; }
for n in "${NODES[@]}"; do
  ! docker volume inspect "pemtest-${n}_pgdata-a-pre-utf8-$RUN2" >/dev/null 2>&1 || fail "case 6: $n kept volume still there"
  ! ls "$WORK/state-$n/$RUN2/"db-*.dump >/dev/null 2>&1 || fail "case 6: $n dumps still there"
done
[[ ! -d "$WORK/founder/pgbackrest-repo.pre-utf8-$RUN2" ]] || fail "case 6: old repository still there"
grep -q "pemtest-founder_pgdata-a-pre-utf8-$RUN" "$WORK/case6.log" || fail "case 6: preflight did not report the rolled-back run's copy"
docker volume inspect "pemtest-founder_pgdata-a-pre-utf8-$RUN" >/dev/null || fail "case 6: the rolled-back run's copy is already gone"
run_play "$WORK/case6d.log" -e postgres_encoding_migrate_action=cleanup -e postgres_encoding_migrate_confirm="$RUN" \
  || { tail -40 "$WORK/case6d.log"; fail "case 6: cleanup of the rolled-back run"; }
for n in "${NODES[@]}"; do
  ! docker volume inspect "pemtest-${n}_pgdata-a-pre-utf8-$RUN" >/dev/null 2>&1 || fail "case 6: $n rolled-back copy still there"
  [[ -f "$WORK/state-$n/$RUN/cleaned" ]] || fail "case 6: $n rolled-back run not marked cleaned"
done
[[ ! -d "$WORK/founder/pgbackrest-repo.utf8-$RUN" ]] || fail "case 6: the rolled-back run's set-aside repository is still there"
[[ "$(pg founder netbird -c "SELECT pg_encoding_to_char(encoding) FROM pg_database WHERE datname='netbird'")" == UTF8 ]] || fail "case 6: cleanup touched the live cluster"
run_play "$WORK/status.log" -e postgres_encoding_migrate_action=status || fail "status"
grep -q "run $RUN: cleaned" "$WORK/status.log" || fail "status output"
echo "  repaired eté, valid text untouched; new run id after the rollback; cleanup of both runs removed their volumes, repositories and dumps"

say "PASS (logs in $WORK)"
