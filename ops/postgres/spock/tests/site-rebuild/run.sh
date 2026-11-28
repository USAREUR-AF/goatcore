#!/usr/bin/env bash
# shellcheck disable=SC2329  # cleanup runs from the EXIT trap; the predicates run through wait_for
# The rebuild of a site whose Spock slots were invalidated, on three
# postgres-spock containers (docs/operations/spock-site-rebuild.md).
#
# Three sites replicate three tables in a full mesh, as spock-mesh-peering
# wires them: nets, which every parent references (as every Kratos table
# references networks), parent, and child, with a foreign key to parent. Each
# site also has a table it does not replicate, sess, with a foreign key to
# parent (as Kratos's sessions reference identities). A is the founder, B and
# C join it. B holds 5 sess rows. B is taken off the network (docker network disconnect: it
# keeps running and takes its own writes, as a joiner site cut off from the
# others does). While B is away:
#   - B writes 10 parents and 10 children of its own: rows only B holds,
#     which the rebuild must not lose;
#   - B changes a row it had from A: the founder's version replaces it, and
#     B's is handed back for review;
#   - A deletes a parent (and its child) that B still holds: the rebuild must
#     not bring it back;
#   - A writes 20 parents and C writes 5, which B has not seen.
# A and B then each write WAL into a table they do not replicate until the slot
# A keeps for B, and B's slots for A and C, read wal_status = lost (the cap is
# CAP, 64MB by default, set on every site).
#
# B returns. The detector must name the three lost slots (GB-387). B is then
# rebuilt from A with spock-reseed.py's eight steps, as `goatbuilder join-node
# --reseed` runs them: the export step must refuse (GB-388) until the run
# accepts the export's digest, and the export must hold B's 20 rows to replay,
# its changed row and the row A deleted for review, and name sess (5 rows) as
# emptied by the truncate. nets, the same on A and B, must be left alone. C,
# whose subscription from B is lost too, is rebuilt next, from A, with B as a
# peer.
#
# Exit 0 when, at the end, the three sites hold the same rows in both tables
# (count and digest), B's 20 rows are on all three, A's deleted row is on none,
# the changed row reads A's version everywhere and is in B's hand-back file,
# nets still holds its one row everywhere, B's sess is empty, a write on each
# site reaches the other two, and the detector finds no lost slot on B or C.
# 1 otherwise.
#
#   IMAGE      postgres-spock image built from ops/postgres/spock (required)
#   HELPER     ops/scripts/spock-reseed.py (default: the one in this checkout)
#   CAP        max_slot_wal_keep_size on every site (default 64MB)
#   OUT        a folder for the record (default: a new temporary folder):
#              events.txt, each step's output, the journals, export and
#              hand-back files, and each site's Postgres log
#   HARNESS_PFX / HARNESS_NET  container prefix and network name
#   KEEP=1     leave the containers up
#
# A run writes a few hundred megabytes of WAL on two containers and takes a
# few minutes. It is not in CI. Linux Docker only, as every multi-site test in
# this directory.

set -euo pipefail
cd "$(dirname "$0")"

IMAGE="${IMAGE:?set IMAGE to a postgres-spock image}"
HELPER="${HELPER:-$(cd ../../../../scripts && pwd)/spock-reseed.py}"
CAP="${CAP:-64MB}"
PFX="${HARNESS_PFX:-srb}"
NET="${HARNESS_NET:-site-rebuild-net}"
PW="partition"
DB=netbird
OUT="${OUT:-$(mktemp -d)}"
mkdir -p "$OUT/logs"
EV="$OUT/events.txt"
: > "$EV"
ev() { echo "$*" | tee -a "$EV"; }
now() { date -u +%Y-%m-%dT%H:%M:%SZ; }
SITES="a b c"

q() { # q <site> <db> <sql>
  docker exec "$PFX-$1" psql -U postgres -d "$2" -v ON_ERROR_STOP=1 -tAXq -c "$3"
}
psql_of() { echo "docker exec -i $PFX-$1 psql -U postgres"; }

down() {
  local s
  for s in $SITES; do docker rm -fv "$PFX-$s" >/dev/null 2>&1 || true; done
  docker network rm "$NET" >/dev/null 2>&1 || true
}
cleanup() {
  local s
  for s in $SITES; do docker logs "$PFX-$s" > "$OUT/logs/$s.log" 2>&1 || true; done
  if [[ "${KEEP:-0}" == 1 ]]; then echo "KEEP=1, leaving $PFX-a $PFX-b $PFX-c up"; return; fi
  down
}
trap cleanup EXIT
down

wait_ready() {
  for _ in $(seq 1 90); do
    docker exec "$1" pg_isready -h 127.0.0.1 -p 5432 -U postgres >/dev/null 2>&1 && return 0
    sleep 2
  done
  echo "FAIL: $1 never accepted a connection"; docker logs "$1" 2>&1 | tail -20; return 1
}

wait_for() { # wait_for <seconds> <description> <command...>: until the command succeeds
  local secs="$1" what="$2"
  shift 2
  for _ in $(seq 1 "$secs"); do
    "$@" >/dev/null 2>&1 && return 0
    sleep 1
  done
  ev "FAIL: $what, after ${secs}s"
  return 1
}

count_is() { [[ "$(q "$1" "$DB" "SELECT count(*) FROM $2;")" == "$3" ]]; }
sum_of() { q "$1" "$DB" "SELECT count(*) || ' ' || coalesce(md5(string_agg(md5(x::text), ',' ORDER BY md5(x::text))), '') FROM $2 x;"; }
same_everywhere() { # same_everywhere <table>
  local a b c
  a="$(sum_of a "$1")"; b="$(sum_of b "$1")"; c="$(sum_of c "$1")"
  [[ "$a" == "$b" && "$b" == "$c" ]]
}

# One helper step for a site. Its output goes to $OUT/<site>-<step>.txt and
# events.txt; the exit status is returned.
step() { # step <site> <step> [extra args...]
  local s="$1" st="$2" rc=0
  shift 2
  python3 "$HELPER" "$st" --node-psql "$(psql_of "$s")" --site-psql "$(psql_of a)" \
    --journal "$OUT/$s-journal.json" --wait 300 "$@" > "$OUT/$s-$st.txt" 2>&1 || rc=$?
  sed "s/^/  $s $st: /" "$OUT/$s-$st.txt" >> "$EV"
  ev "$s step $st: exit $rc"
  return "$rc"
}

ev "image $IMAGE ($(docker image inspect "$IMAGE" --format '{{.Id}}'))"
ev "helper $HELPER (sha256 $(sha256sum "$HELPER" | cut -d' ' -f1))"
ev "started $(now)"
docker network create "$NET" >/dev/null
for s in $SITES; do
  docker run -d --name "$PFX-$s" --network "$NET" -e POSTGRES_PASSWORD="$PW" "$IMAGE" \
    postgres -c config_file=/etc/postgresql/postgresql.conf -c cluster_name="site_$s" \
    -c max_slot_wal_keep_size="$CAP" -c max_wal_size="$CAP" -c min_wal_size=32MB >/dev/null
done
for s in $SITES; do wait_ready "$PFX-$s"; done
ev "max_slot_wal_keep_size = $(q a postgres "SHOW max_slot_wal_keep_size;") on every site"

for s in $SITES; do
  q "$s" postgres "CREATE ROLE replicator WITH LOGIN SUPERUSER PASSWORD '$PW';"
  docker exec --user root "$PFX-$s" bash -c \
    "echo 'host all replicator 0.0.0.0/0 scram-sha-256' >> /etc/postgresql/pg_hba.conf"
  q "$s" postgres "SELECT pg_reload_conf();" >/dev/null
  # DDL is not replicated: every site creates the tables.
  q "$s" "$DB" "CREATE TABLE nets (id int PRIMARY KEY, v text NOT NULL);
                CREATE TABLE parent (id uuid PRIMARY KEY DEFAULT gen_random_uuid(), nid int NOT NULL DEFAULT 1 REFERENCES nets(id),
                                     site text NOT NULL, v text NOT NULL);
                CREATE TABLE child (id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
                                    parent_id uuid NOT NULL REFERENCES parent(id) ON DELETE CASCADE, v text NOT NULL);
                CREATE TABLE sess (id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
                                   parent_id uuid NOT NULL REFERENCES parent(id) ON DELETE CASCADE);
                CREATE TABLE load (id bigserial PRIMARY KEY, pad text NOT NULL);"
  q "$s" "$DB" "SELECT spock.node_create('site_$s', 'host=$PFX-$s dbname=$DB user=replicator password=$PW');" >/dev/null
  q "$s" "$DB" "SELECT spock.repset_add_table('default', 'nets'); SELECT spock.repset_add_table('default', 'parent');
                SELECT spock.repset_add_table('default', 'child');" >/dev/null
done
# A full mesh, as spock-mesh-peering wires it: no initial copy, no forwarded origins.
for sub in $SITES; do
  for prov in $SITES; do
    [[ "$sub" == "$prov" ]] && continue
    q "$sub" "$DB" "SELECT spock.sub_create(
        subscription_name := 'sub_${prov}_to_${sub}',
        provider_dsn      := 'host=$PFX-$prov dbname=$DB user=replicator password=$PW',
        replication_sets  := ARRAY['default'],
        synchronize_data  := false,
        forward_origins   := ARRAY[]::text[]);" >/dev/null
  done
done
replicating() { [[ "$(q "$1" "$DB" "SELECT count(*) FROM spock.sub_show_status() WHERE status = 'replicating';")" == 2 ]]; }
for s in $SITES; do wait_for 60 "site $s's two subscriptions replicating" replicating "$s"; done

q a "$DB" "INSERT INTO nets VALUES (1, 'the one network');
           INSERT INTO parent (site, v) SELECT 'a-seed', g::text FROM generate_series(1, 50) g;
           INSERT INTO child (parent_id, v) SELECT id, 'seed' FROM parent;"
for s in b c; do wait_for 60 "the seed rows on $s" count_is "$s" child 50; done
q b "$DB" "INSERT INTO sess (parent_id) SELECT id FROM parent ORDER BY id LIMIT 5;"
ev "seeded: 1 net, 50 parents and 50 children on every site; 5 sess rows on B, not replicated"

# B leaves the network, and restarts while it is off. Without the restart, a
# change already sent on a connection that outlives the cut is delivered when
# the network returns (TCP retransmits it for about 15 minutes), before the
# sender learns its slot is lost: a lab cut of seconds would carry some writes
# across. The restart drops those connections, as a cut of more than
# TCP's retransmission timeout does.
docker network disconnect "$NET" "$PFX-b"
docker restart "$PFX-b" >/dev/null
wait_ready "$PFX-b"
ev "B cut off and restarted at $(now)"
q b "$DB" "INSERT INTO parent (site, v) SELECT 'b-island', g::text FROM generate_series(1, 10) g;
           INSERT INTO child (parent_id, v) SELECT id, 'b-island' FROM parent WHERE site = 'b-island';"
CHANGED="$(q b "$DB" "SELECT id FROM parent WHERE site = 'a-seed' ORDER BY id LIMIT 1;")"
CHANGED_V="$(q a "$DB" "SELECT v FROM parent WHERE id = '$CHANGED';")"
q b "$DB" "UPDATE parent SET v = 'changed at b' WHERE id = '$CHANGED';"
DELETED="$(q a "$DB" "SELECT id FROM parent WHERE site = 'a-seed' ORDER BY id DESC LIMIT 1;")"
q a "$DB" "DELETE FROM parent WHERE id = '$DELETED';"
q a "$DB" "INSERT INTO parent (site, v) SELECT 'a-missed', g::text FROM generate_series(1, 20) g;"
q c "$DB" "INSERT INTO parent (site, v) SELECT 'c-during', g::text FROM generate_series(1, 5) g;"
ev "while B was away: B wrote 10 parents and 10 children and changed parent $CHANGED; A deleted parent $DELETED and wrote 20 parents; C wrote 5"

lost() { [[ "$(q "$1" postgres "SELECT coalesce(wal_status, '') FROM pg_replication_slots WHERE slot_name = '$2';")" == lost ]]; }
T0="$(date +%s)"
rounds=0
until lost a spk_netbird_site_a_sub_a_to_b && lost b spk_netbird_site_b_sub_b_to_a && lost b spk_netbird_site_b_sub_b_to_c; do
  rounds=$((rounds + 1))
  [[ "$rounds" -le 60 ]] || { ev "FAIL: the slots were not lost after 60 rounds of WAL"; exit 1; }
  for s in a b; do
    q "$s" "$DB" "INSERT INTO load (pad) SELECT (SELECT string_agg(md5(random()::text || g::text || i::text), '') FROM generate_series(1, 256) i)
                  FROM generate_series(1, 2000) g;"
    q "$s" postgres "CHECKPOINT;"
  done
done
ev "slots lost after $rounds rounds ($(( $(date +%s) - T0 ))s): A's for B, B's for A and for C"
for s in $SITES; do
  ev "  slots on $s: $(q "$s" postgres "SELECT string_agg(slot_name || '=' || coalesce(wal_status, ''), ' ' ORDER BY slot_name) FROM pg_replication_slots;")"
done

docker network connect "$NET" "$PFX-b"
ev "B back at $(now)"
sleep 10

# The detector, for B, with A and C.
rc=0
python3 "$HELPER" slots --node-psql "$(psql_of b)" --site-psql "$(psql_of a)" --peer-psql "c=$(psql_of c)" \
  > "$OUT/b-slots-before.txt" 2>&1 || rc=$?
sed 's/^/  b slots: /' "$OUT/b-slots-before.txt" >> "$EV"
ev "detector on B: exit $rc, $(grep -c 'cannot recover:' "$OUT/b-slots-before.txt" || true) lines that cannot recover"
ok=1
if [[ "$rc" != 1 ]] || ! grep -q '^GB-387: 3 Spock slot' "$OUT/b-slots-before.txt"; then ev "FAIL: the detector did not name the three lost slots"; ok=0; fi

# B's rebuild. The export step refuses until its digest is accepted.
R0="$(date +%s)"
step b lost --peer-psql "c=$(psql_of c)" || { ev "FAIL: B's lost step"; exit 1; }
step b caught-up || { ev "FAIL: B's caught-up step"; exit 1; }
rc=0; step b export || rc=$?
if [[ "$rc" != 1 ]] || ! grep -q '^GB-388: ' "$OUT/b-export.txt"; then ev "FAIL: B's export did not refuse with GB-388 (exit $rc)"; exit 1; fi
EXPORT="$OUT/b-journal.export.json"
DIGEST="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["digest"])' "$EXPORT")"
read -r N_REPLAY N_REVIEW <<<"$(python3 - "$EXPORT" <<'EOF'
import json, sys
d = json.load(open(sys.argv[1]))["databases"]["netbird"]["site_only"]
print(sum(len(t["replay"]) for t in d.values()), sum(len(t["review"]) for t in d.values()))
EOF
)"
ev "B's export: $N_REPLAY rows to replay, $N_REVIEW for review, digest $DIGEST"
[[ "$N_REPLAY" == 20 && "$N_REVIEW" == 3 ]] || { ev "FAIL: B's export holds $N_REPLAY to replay and $N_REVIEW for review, not 20 and 3 (the changed parent, the deleted parent and its child)"; ok=0; }
EMPTIED="$(python3 -c 'import json,sys; print(json.dumps(json.load(open(sys.argv[1]))["databases"]["netbird"]["plan"], sort_keys=True))' "$EXPORT")"
ev "B's copy plan: $EMPTIED"
[[ "$EMPTIED" == '{"copy": ["child", "parent"], "emptied": {"sess": 5}, "same": ["nets"]}' ]] || { ev "FAIL: B's plan is not: copy child and parent, empty sess (5 rows), leave nets"; ok=0; }
for st in export size reseed synced replay converged; do
  step b "$st" --accept "$DIGEST" --peer-psql "c=$(psql_of c)" || { ev "FAIL: B's $st step"; exit 1; }
done
ev "B rebuilt in $(( $(date +%s) - R0 ))s"
HANDBACK="$OUT/b-journal.handback.json"
grep -q "$CHANGED" "$HANDBACK" || { ev "FAIL: B's changed row is not in its hand-back file"; ok=0; }

# C's subscription from B is lost (B's slot for it): C is rebuilt from A, with B as a peer.
rc=0
python3 "$HELPER" slots --node-psql "$(psql_of c)" --site-psql "$(psql_of a)" --peer-psql "b=$(psql_of b)" \
  > "$OUT/c-slots-before.txt" 2>&1 || rc=$?
sed 's/^/  c slots: /' "$OUT/c-slots-before.txt" >> "$EV"
ev "detector on C: exit $rc"
R0="$(date +%s)"
for st in lost caught-up export size reseed synced replay converged; do
  step c "$st" --peer-psql "b=$(psql_of b)" || { ev "FAIL: C's $st step"; exit 1; }
done
ev "C rebuilt in $(( $(date +%s) - R0 ))s"

[[ "$(q b "$DB" "SELECT count(*) FROM sess;")" == 0 ]] || { ev "FAIL: B's sess was not emptied by the truncate"; ok=0; }

# Every site writes once more; each write reaches the other two.
for s in $SITES; do q "$s" "$DB" "INSERT INTO parent (site, v) VALUES ('$s-after', 'after the rebuild');"; done
for t in nets parent child; do
  wait_for 60 "$t the same on the three sites" same_everywhere "$t" || ok=0
  ev "$t: A $(sum_of a "$t") | B $(sum_of b "$t") | C $(sum_of c "$t")"
done
for s in $SITES; do
  n="$(q "$s" "$DB" "SELECT count(*) FROM parent WHERE site = 'b-island';") $(q "$s" "$DB" "SELECT count(*) FROM child WHERE v = 'b-island';")"
  [[ "$n" == "10 10" ]] || { ev "FAIL: site $s holds $n of B's 10 parents and 10 children"; ok=0; }
  [[ "$(q "$s" "$DB" "SELECT count(*) FROM parent WHERE id = '$DELETED';")" == 0 ]] || { ev "FAIL: A's deleted parent is back on $s"; ok=0; }
  [[ "$(q "$s" "$DB" "SELECT v FROM parent WHERE id = '$CHANGED';")" == "$CHANGED_V" ]] || { ev "FAIL: the changed parent on $s does not read A's version"; ok=0; }
  [[ "$(q "$s" "$DB" "SELECT count(*) FROM parent WHERE site LIKE '%-after';")" == 3 ]] || { ev "FAIL: site $s does not hold the three writes made after the rebuild"; ok=0; }
  [[ "$(q "$s" "$DB" "SELECT count(*) FROM nets;")" == 1 ]] || { ev "FAIL: site $s does not hold the one net"; ok=0; }
done
for s in b c; do
  other=c; [[ "$s" == c ]] && other=b
  rc=0
  python3 "$HELPER" slots --node-psql "$(psql_of "$s")" --site-psql "$(psql_of a)" --peer-psql "$other=$(psql_of "$other")" \
    > "$OUT/$s-slots-after.txt" 2>&1 || rc=$?
  sed "s/^/  $s slots: /" "$OUT/$s-slots-after.txt" >> "$EV"
  [[ "$rc" == 0 ]] || { ev "FAIL: the detector finds a lost slot on $s after the rebuild (exit $rc)"; ok=0; }
done
ev "finished $(now)"
[[ "$ok" == 1 ]] && ev "PASS" && exit 0
ev "FAIL"
exit 1
