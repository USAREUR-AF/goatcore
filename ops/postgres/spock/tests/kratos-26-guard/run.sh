#!/usr/bin/env bash
# The ADR 1193 revocation guard across the Kratos 1.3.1 -> 26.2.0 migration.
#
# Kratos 26.2.0 backfills identity_credential_identifiers.identity_id with an
# UPDATE over every row, then sets it NOT NULL with a foreign key. The guard's
# BEFORE INSERT OR UPDATE trigger (goat_guard) on that table returns NULL, so
# skips the row, when the owner is a deleted identity or the credential is
# missing. This test runs the REAL Kratos migrations on a postgres-spock
# database that carries the guard and real Kratos-written rows, and proves:
#
#   1. clean   on a consistent database (one identity deleted, one set
#              inactive, through Kratos), roles/ory/files/kratos-backfill-blockers.sql
#              counts 0 everywhere; the 26.2.0 migrations complete; no
#              identifier has a NULL identity_id and each matches its
#              credential's owner; every guard and capture trigger is still
#              present and ENABLE ALWAYS.
#   2. guard   on the 26.2.0 schema: a deleted identity's row and its child
#              rows re-inserted (as a late replicated write would be) are
#              dropped; an inactive identity written active stays inactive; a
#              new identity created and deleted through Kratos 26.2.0 is
#              recorded and its re-insert dropped.
#   3. hazard  on a copy of the 1.3.1 database with a deleted-identity
#              revocation recorded while its credentials still exist, and
#              with a child row left behind by a delete that ran with
#              foreign-key triggers off (as Spock applies one): the blocker
#              query counts both, and the 26.2.0 migration fails on that copy.
#              This is why ory-kratos-upgrade-window.yml refuses to start
#              while the blocker query counts anything.
#
#   IMAGE=postgres-spock:smoke ./run.sh
#
# Substrate: Linux Docker (CI). Needs network access to pull the Kratos images.

set -euo pipefail
cd "$(dirname "$0")"

IMAGE="${IMAGE:?set IMAGE to a postgres-spock image}"
# The version upgraded FROM, on purpose not the manifest version.
KRATOS_OLD="${KRATOS_OLD:-oryd/kratos:v1.3.1@sha256:fe2428f103a6240c064b6ea77d3088610865c55fcebccb30355857eccdc25b4b}"  # upstream-pin: allow
KRATOS_NEW="${KRATOS_NEW:-oryd/kratos:v26.2.0@sha256:2a13bb8d362c7a7ae33bd7c0f5168aee46921f15c916a06346db91c06dc76643}"
GUARD_SQL="../../revocation-guard/kratos.sql"
BLOCKERS_SQL="../../../../ansible/roles/ory/files/kratos-backfill-blockers.sql"
NET="kratos26-guard-net"
PG="kratos26-guard-pg"
KR="kratos26-guard-kratos"
PW="kratos26guard"
ADMIN_PORT="${ADMIN_PORT:-14434}"
ADMIN="http://127.0.0.1:${ADMIN_PORT}"
WORK="$(mktemp -d)"
fails=0

cleanup() {
  [[ "${KEEP:-0}" == 1 ]] && { echo "KEEP=1, leaving $PG $KR running"; return; }
  docker rm -fv "$KR" "$PG" >/dev/null 2>&1 || true
  docker network rm "$NET" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT
cleanup; WORK="$(mktemp -d)"

ok()   { echo "  PASS  $*"; }
bad()  { echo "  FAIL  $*"; fails=$((fails + 1)); }
q()    { docker exec "$PG" psql -U postgres -d "$1" -v ON_ERROR_STOP=1 -tAXq -c "$2"; }
want() { # want <db> <expected> <description> <sql>
  local got; got="$(q "$1" "$4")"
  if [[ "$got" == "$2" ]]; then ok "$3"; else bad "$3: got '$got', want '$2'"; fi
}
dsn()  { echo "postgres://postgres:${PW}@${PG}:5432/$1?sslmode=disable"; }
migrate() { # migrate <image> <db> <sql subcommand...>
  local img="$1" db="$2"; shift 2
  docker run --rm --network "$NET" -e DSN="$(dsn "$db")" -v "$WORK/cfg:/etc/config:ro" \
    "$img" migrate -c /etc/config/kratos.yaml sql "$@"
}
serve() { # serve <image>
  docker rm -fv "$KR" >/dev/null 2>&1 || true
  docker run -d --name "$KR" --network "$NET" -e DSN="$(dsn kratos)" -v "$WORK/cfg:/etc/config:ro" \
    -p "127.0.0.1:${ADMIN_PORT}:4434" "$1" serve -c /etc/config/kratos.yaml --dev >/dev/null
  for _ in $(seq 1 90); do
    curl -fsS -o /dev/null "$ADMIN/admin/health/ready" 2>/dev/null && return 0
    sleep 1
  done
  docker logs --tail 60 "$KR"; echo "::error::kratos $1 did not become ready"; exit 1
}
create_identity() { # create_identity <email> -> id
  curl -fsS -X POST "$ADMIN/admin/identities" -H 'Content-Type: application/json' -d \
    "{\"schema_id\":\"default\",\"traits\":{\"email\":\"$1\"},
      \"credentials\":{\"password\":{\"config\":{\"password\":\"guard-test-$RANDOM-Aa1!x\"}}},
      \"verifiable_addresses\":[{\"value\":\"$1\",\"verified\":true,\"via\":\"email\",\"status\":\"completed\"}],
      \"recovery_addresses\":[{\"value\":\"$1\",\"via\":\"email\"}]}" \
    | python3 -c 'import sys, json; print(json.load(sys.stdin)["id"])'
}
blockers() { docker exec -i "$PG" psql -U postgres -d "$1" -v ON_ERROR_STOP=1 -tAq < "$BLOCKERS_SQL"; }

mkdir -p "$WORK/cfg"
cat > "$WORK/cfg/identity.schema.json" <<'EOF'
{"$id":"default","$schema":"http://json-schema.org/draft-07/schema#","type":"object",
 "properties":{"traits":{"type":"object","properties":{"email":{"type":"string","format":"email",
   "ory.sh/kratos":{"credentials":{"password":{"identifier":true}},"verification":{"via":"email"},"recovery":{"via":"email"}}}},
   "required":["email"],"additionalProperties":false}}}
EOF
cat > "$WORK/cfg/kratos.yaml" <<'EOF'
serve:
  public:
    base_url: http://127.0.0.1:4433/
  admin:
    base_url: http://127.0.0.1:4434/
identity:
  default_schema_id: default
  schemas:
    - id: default
      url: file:///etc/config/identity.schema.json
selfservice:
  default_browser_return_url: http://127.0.0.1:4455/
secrets:
  cookie: ["kratos26-guard-cookie-secret-0123456789"]
  cipher: ["kratos26guardcipher32bytesxxxxxx"]
hashers:
  algorithm: bcrypt
  bcrypt:
    cost: 4
courier:
  smtp:
    connection_uri: smtps://localhost:1025/?disable_starttls=true
EOF

echo "== postgres-spock"
docker network create "$NET" >/dev/null
docker run -d --name "$PG" --network "$NET" -e POSTGRES_PASSWORD="$PW" "$IMAGE" \
  postgres -c config_file=/etc/postgresql/postgresql.conf -c cluster_name=site_a >/dev/null
for _ in $(seq 1 90); do
  docker exec "$PG" pg_isready -U postgres -q 2>/dev/null && q postgres "SELECT 1" >/dev/null 2>&1 && break
  sleep 1
done
q postgres "SELECT 1 FROM pg_database WHERE datname = 'kratos'" | grep -q 1 || q postgres "CREATE DATABASE kratos;"
docker pull -q "$KRATOS_OLD" >/dev/null
docker pull -q "$KRATOS_NEW" >/dev/null

echo "== Kratos 1.3.1 schema and rows written by Kratos 1.3.1"
migrate "$KRATOS_OLD" kratos -e --yes >/dev/null
serve "$KRATOS_OLD"
KEEP_ID="$(create_identity keep@example.test)"
GONE_ID="$(create_identity gone@example.test)"
IDLE_ID="$(create_identity idle@example.test)"
echo "  identities keep=$KEEP_ID gone=$GONE_ID idle=$IDLE_ID"

echo "== install the revocation guard (as on a goatnet before the upgrade)"
docker exec -i "$PG" psql -U postgres -d kratos -v ON_ERROR_STOP=1 -q < "$GUARD_SQL"

echo "== revoke through Kratos: delete one identity, set one inactive"
curl -fsS -X DELETE "$ADMIN/admin/identities/$GONE_ID" -o /dev/null
q kratos "UPDATE identities SET state = 'inactive', state_changed_at = now(), updated_at = now() WHERE id = '$IDLE_ID';"
want kratos t "deletion recorded in goat_revocations" "SELECT goat_revoked('kratos.identity.deleted', '$GONE_ID')"
want kratos t "inactivation recorded in goat_revocations" "SELECT goat_revoked('kratos.identity', '$IDLE_ID')"
want kratos 0 "the deleted identity's identifiers went with it (Kratos cascade)" \
  "SELECT count(*) FROM identity_credential_identifiers ici JOIN identity_credentials ic ON ic.id = ici.identity_credential_id WHERE ic.identity_id = '$GONE_ID'"
docker rm -fv "$KR" >/dev/null

echo "== copy the 1.3.1 database for the hazard case (part 3)"
q postgres "CREATE DATABASE kratos_hazard;"
docker exec "$PG" bash -c "pg_dump -U postgres -d kratos | psql -U postgres -d kratos_hazard -v ON_ERROR_STOP=1 -q" >/dev/null

echo "== 1. clean: blockers, migrate to 26.2.0, check"
IDENT_BEFORE="$(q kratos 'SELECT count(*) FROM identity_credential_identifiers')"
got="$(blockers kratos | tr '\n' ' ')"
if [[ "$got" == "identifier_without_credential|0 identifier_owner_missing|0 identifier_owner_deleted|0 session_device_orphaned|0 " ]]; then
  ok "blocker query counts nothing on a consistent database"
else
  bad "blocker query on a consistent database: $got"
fi
if migrate "$KRATOS_NEW" kratos up -e --yes > "$WORK/migrate-clean.log" 2>&1; then
  ok "Kratos 26.2.0 migrations complete with the guard installed"
else
  tail -30 "$WORK/migrate-clean.log"; bad "Kratos 26.2.0 migrations failed on a consistent database"
fi
want kratos "$IDENT_BEFORE" "no identifier lost in the backfill" "SELECT count(*) FROM identity_credential_identifiers"
want kratos 0 "no identifier left with a NULL identity_id" \
  "SELECT count(*) FROM identity_credential_identifiers WHERE identity_id IS NULL"
want kratos 0 "every identifier's identity_id is its credential's owner" \
  "SELECT count(*) FROM identity_credential_identifiers ici JOIN identity_credentials ic ON ic.id = ici.identity_credential_id WHERE ici.identity_id <> ic.identity_id"
want kratos NO "identity_id is NOT NULL" \
  "SELECT is_nullable FROM information_schema.columns WHERE table_name = 'identity_credential_identifiers' AND column_name = 'identity_id'"
for t in identities identity_credentials identity_credential_identifiers identity_verifiable_addresses identity_recovery_addresses; do
  want kratos A "goat_guard on $t still present and ENABLE ALWAYS" \
    "SELECT tgenabled FROM pg_trigger WHERE tgrelid = 'public.$t'::regclass AND tgname = 'goat_guard'"
done
want kratos 1 "goat_capture on identities still present" \
  "SELECT count(*) FROM pg_trigger WHERE tgrelid = 'public.identities'::regclass AND tgname = 'goat_capture'"

echo "== 2. guard on the 26.2.0 schema"
NID="$(q kratos "SELECT nid FROM identities WHERE id = '$KEEP_ID'")"
q kratos "INSERT INTO identities (id, nid, schema_id, traits, state, created_at, updated_at)
          VALUES ('$GONE_ID', '$NID', 'default', '{\"email\":\"gone@example.test\"}', 'active', now(), now());"
want kratos 0 "a deleted identity's row re-inserted is dropped" "SELECT count(*) FROM identities WHERE id = '$GONE_ID'"
CRED_TYPE="$(q kratos "SELECT id FROM identity_credential_types WHERE name = 'password'")"
q kratos "INSERT INTO identity_credentials (id, config, identity_credential_type_id, identity_id, nid, created_at, updated_at)
          VALUES (gen_random_uuid(), '{}', '$CRED_TYPE', '$GONE_ID', '$NID', now(), now());"
want kratos 0 "a deleted identity's credential re-inserted is dropped" \
  "SELECT count(*) FROM identity_credentials WHERE identity_id = '$GONE_ID'"
KEEP_CRED="$(q kratos "SELECT id FROM identity_credentials WHERE identity_id = '$KEEP_ID' LIMIT 1")"
q kratos "INSERT INTO identity_credential_identifiers (id, identifier, identity_credential_id, identity_credential_type_id, identity_id, nid, created_at, updated_at)
          VALUES (gen_random_uuid(), 'late@example.test', '$KEEP_CRED', '$CRED_TYPE', '$KEEP_ID', '$NID', now(), now());"
want kratos 1 "a live identity's new identifier is accepted (26.2.0 column set)" \
  "SELECT count(*) FROM identity_credential_identifiers WHERE identifier = 'late@example.test'"
q kratos "UPDATE identities SET state = 'active', updated_at = now() WHERE id = '$IDLE_ID';"
want kratos inactive "an inactive identity written active stays inactive" "SELECT state FROM identities WHERE id = '$IDLE_ID'"

serve "$KRATOS_NEW"
FRESH_ID="$(create_identity fresh@example.test)"
want kratos 1 "Kratos 26.2.0 writes an identifier with identity_id through the guard" \
  "SELECT count(*) FROM identity_credential_identifiers WHERE identity_id = '$FRESH_ID'"
curl -fsS -X DELETE "$ADMIN/admin/identities/$FRESH_ID" -o /dev/null
want kratos t "a deletion through Kratos 26.2.0 is recorded" "SELECT goat_revoked('kratos.identity.deleted', '$FRESH_ID')"
q kratos "INSERT INTO identities (id, nid, schema_id, traits, state, created_at, updated_at)
          VALUES ('$FRESH_ID', '$NID', 'default', '{\"email\":\"fresh@example.test\"}', 'active', now(), now());"
want kratos 0 "its re-insert is dropped" "SELECT count(*) FROM identities WHERE id = '$FRESH_ID'"
code="$(curl -s -o /dev/null -w '%{http_code}' "$ADMIN/admin/identities/$GONE_ID")"
if [[ "$code" == 404 ]]; then ok "Kratos 26.2.0 answers 404 for the deleted identity"; else bad "GET deleted identity: HTTP $code"; fi
docker rm -fv "$KR" >/dev/null

echo "== 3. hazard: rows the backfill cannot fill stop the migration"
# A deleted-identity revocation that landed while the identity's credentials
# still exist (the apply that removes them did not run).
q kratos_hazard "ALTER TABLE goat_revocations DISABLE TRIGGER goat_revocations_on_insert;
                 SELECT goat_revoke('kratos.identity.deleted', '$KEEP_ID', 'test');
                 ALTER TABLE goat_revocations ENABLE ALWAYS TRIGGER goat_revocations_on_insert;"
# An identity removed the way Spock applies a delete: foreign-key triggers
# off, so its credentials and identifiers stay behind.
q kratos_hazard "SET session_replication_role = replica;
                 DELETE FROM identities WHERE id = '$IDLE_ID';"
got="$(blockers kratos_hazard | tr '\n' ' ')"
case "$got" in
  *"identifier_owner_deleted|0 "*) bad "blocker query missed the deleted owner: $got" ;;
  *"identifier_owner_missing|0 "*) bad "blocker query missed the missing owner: $got" ;;
  *) ok "blocker query counts both: $got" ;;
esac
if migrate "$KRATOS_NEW" kratos_hazard up -e --yes > "$WORK/migrate-hazard.log" 2>&1; then
  bad "Kratos 26.2.0 migrated a database the blocker query flagged (the preflight would be unnecessary)"
else
  # Where it stopped: the identity_id backfills (20251105000000000001/2) ran,
  # the NOT NULL / foreign-key step (20251105000000000003) did not.
  at="$(q kratos_hazard "SELECT coalesce(max(version), '') FROM schema_migration WHERE version >= '20251105000000000000'")"
  nulls="$(q kratos_hazard "SELECT count(*) FROM identity_credential_identifiers WHERE identity_id IS NULL")"
  if [[ ( "$at" == 20251105000000000001 || "$at" == 20251105000000000002 ) && "$nulls" -ge 1 ]]; then
    ok "Kratos 26.2.0 migration stops at the identity_id NOT NULL / foreign-key step ($nulls identifier(s) left NULL by the guard)"
  else
    grep -i -m3 -E 'error|violat|null value' "$WORK/migrate-hazard.log" | cut -c1-300 || true
    bad "migration failed elsewhere: last applied >= 20251105 is '$at', NULL identifiers $nulls"
  fi
fi

echo
if [[ "$fails" -ne 0 ]]; then echo "kratos-26-guard: $fails check(s) failed"; exit 1; fi
echo "kratos-26-guard: PASS"
