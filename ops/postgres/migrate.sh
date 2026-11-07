#!/usr/bin/env bash
set -euo pipefail

# goat — Migrate Netbird from SQLite to PostgreSQL
#
# Run ON THE NETHOST (not from LOCAL).
# Prerequisites:
#   - Postgres container running (docker compose up -d in this directory)
#   - pgloader installed (sudo apt-get install -y pgloader)
#   - sqlite3 installed (sudo apt-get install -y sqlite3)
#   - Netbird SQLite databases backed up
#
# Usage: sudo bash migrate.sh <PG_PASSWORD>
#
# Host directory: GOAT_HOST_OPT_DIR (the group_vars goat_host_opt_dir), else the
# parent of the netbird-postgres compose project's working directory, else
# /opt/goat. The script runs as a copied file on the host, so it reads no tree.

if [ $# -lt 1 ]; then
  echo "Usage: sudo bash migrate.sh <PG_PASSWORD>"
  exit 1
fi

PG_PASS="$1"
if [ -z "${GOAT_HOST_OPT_DIR:-}" ]; then
  _pg_compose_dir="$(docker inspect --format '{{ index .Config.Labels "com.docker.compose.project.working_dir" }}' netbird-postgres 2>/dev/null || true)"
  case "$_pg_compose_dir" in /*/*) GOAT_HOST_OPT_DIR="$(dirname "$_pg_compose_dir")" ;; *) GOAT_HOST_OPT_DIR=/opt/goat ;; esac
fi
BACKUP_DIR="$GOAT_HOST_OPT_DIR/postgres-migration/backup"
WORK_DIR="$GOAT_HOST_OPT_DIR/postgres-migration/work"
PGHOST="127.0.0.1"

# Verify Postgres is reachable
if ! docker exec netbird-postgres pg_isready -U netbird >/dev/null 2>&1; then
  echo "ERROR: netbird-postgres container not ready. Is it running?"
  echo "  Start it: cd $GOAT_HOST_OPT_DIR/postgres && docker compose up -d"
  exit 1
fi

# Verify sqlite3 is available
if ! command -v sqlite3 >/dev/null 2>&1; then
  echo "ERROR: sqlite3 not found. Install it: sudo apt-get install -y sqlite3"
  exit 1
fi

echo "Postgres host: $PGHOST (localhost, port-mapped from container)"
echo ""

# -----------------------------------------------
# Step 1: Extract SQLite databases from container
# -----------------------------------------------
echo "[1/6] Extracting SQLite databases..."
mkdir -p "$BACKUP_DIR" "$WORK_DIR"
docker cp netbird-server:/var/lib/netbird/store.db "$BACKUP_DIR/"
docker cp netbird-server:/var/lib/netbird/idp.db "$BACKUP_DIR/" 2>/dev/null || echo "  WARN: idp.db not found (may use different path)"
docker cp netbird-server:/var/lib/netbird/events.db "$BACKUP_DIR/" 2>/dev/null || echo "  WARN: events.db not found (may use different path)"

echo "  Backed up to $BACKUP_DIR/"
ls -la "$BACKUP_DIR/"

# Save encryption key
echo ""
echo "[1b] Saving encryption key..."
ENC_KEY=$(docker exec netbird-server cat /etc/netbird/management.json 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin).get('DataStoreEncryptionKey',''))" 2>/dev/null || echo "")
if [ -n "$ENC_KEY" ]; then
  echo "  Encryption key found: ${ENC_KEY:0:8}..."
  echo "$ENC_KEY" > "$BACKUP_DIR/encryption-key.txt"
else
  echo "  WARN: Could not extract encryption key. Check management.json manually."
fi

# -----------------------------------------------
# Step 2: Pre-process SQLite — convert text booleans to integers
# -----------------------------------------------
# pgloader 3.6.7 (Ubuntu 24.04) does NOT have the sqlite-text-boolean
# transform function. We convert 'true'/'false' text values to 1/0 integers
# in working copies so pgloader can handle them without CAST rules.
echo ""
echo "[2/6] Pre-processing SQLite databases (boolean conversion)..."

# Copy originals to work dir (never modify backups)
cp "$BACKUP_DIR/store.db" "$WORK_DIR/store.db"
[ -f "$BACKUP_DIR/idp.db" ] && cp "$BACKUP_DIR/idp.db" "$WORK_DIR/idp.db"
[ -f "$BACKUP_DIR/events.db" ] && cp "$BACKUP_DIR/events.db" "$WORK_DIR/events.db"

# store.db boolean columns
STORE_BOOL_COLS=(
  "accounts.is_domain_primary_account"
  "accounts.settings_peer_login_expiration_enabled"
  "accounts.settings_peer_inactivity_expiration_enabled"
  "accounts.settings_regular_users_view_blocked"
  "accounts.settings_groups_propagation_enabled"
  "accounts.settings_jwt_groups_enabled"
  "accounts.settings_routing_peer_dns_resolution_enabled"
  "accounts.settings_extra_peer_approval_enabled"
  "accounts.settings_extra_user_approval_required"
  "accounts.settings_lazy_connection_enabled"
)

for col_spec in "${STORE_BOOL_COLS[@]}"; do
  tbl="${col_spec%%.*}"
  col="${col_spec##*.}"
  sqlite3 "$WORK_DIR/store.db" "UPDATE $tbl SET $col = CASE WHEN lower($col) = 'true' THEN 1 WHEN lower($col) = 'false' THEN 0 ELSE $col END;" 2>/dev/null || true
done
echo "  store.db: converted ${#STORE_BOOL_COLS[@]} boolean columns (data)"

echo "  store.db: data conversion done (accounts schema handled separately in step 5)"

# idp.db boolean columns
if [ -f "$WORK_DIR/idp.db" ]; then
  IDP_BOOL_COLS=(
    "client.public"
    "auth_request.force_approval_prompt"
    "auth_request.logged_in"
    "auth_request.claims_email_verified"
    "auth_code.claims_email_verified"
    "refresh_token.claims_email_verified"
  )

  for col_spec in "${IDP_BOOL_COLS[@]}"; do
    tbl="${col_spec%%.*}"
    col="${col_spec##*.}"
    sqlite3 "$WORK_DIR/idp.db" "UPDATE $tbl SET $col = CASE WHEN lower($col) = 'true' THEN 1 WHEN lower($col) = 'false' THEN 0 ELSE $col END;" 2>/dev/null || true
  done
  echo "  idp.db: converted ${#IDP_BOOL_COLS[@]} boolean columns (data)"
fi

# -----------------------------------------------
# Step 3: Drop and recreate Postgres databases (clean slate)
# -----------------------------------------------
echo ""
echo "[3/6] Resetting Postgres databases..."
docker exec netbird-postgres psql -U netbird -d postgres -c "DROP DATABASE IF EXISTS netbird;" 2>/dev/null
docker exec netbird-postgres psql -U netbird -d postgres -c "DROP DATABASE IF EXISTS netbird_idp;" 2>/dev/null
docker exec netbird-postgres psql -U netbird -d postgres -c "DROP DATABASE IF EXISTS netbird_events;" 2>/dev/null
docker exec netbird-postgres psql -U netbird -d postgres -c "CREATE DATABASE netbird;"
docker exec netbird-postgres psql -U netbird -d postgres -c "CREATE DATABASE netbird_idp;"
docker exec netbird-postgres psql -U netbird -d postgres -c "CREATE DATABASE netbird_events;"
echo "  Databases recreated."

# -----------------------------------------------
# Step 4: Migrate events.db (simplest — dry run)
# -----------------------------------------------
echo ""
echo "[4/6] Migrating events.db..."
if [ -f "$WORK_DIR/events.db" ]; then
  pgloader --type sqlite "$WORK_DIR/events.db" \
    "postgresql://netbird:${PG_PASS}@${PGHOST}:5432/netbird_events"
  echo "  events.db migrated."
else
  echo "  Skipped (file not found)."
fi

# -----------------------------------------------
# Step 5: Migrate store.db
# -----------------------------------------------
# pgloader 3.6.7 chokes on accounts table because SQLite stores
# DEFAULT 'false' in a way pgloader can't fix during schema creation.
# Solution: create accounts manually, import its data, then let pgloader
# handle everything else.
echo ""
echo "[5/6] Migrating store.db..."

# 5a: Create accounts table in Postgres with correct schema.
# We use pgloader's own type mapping (seen in its error output) with the
# one bad default fixed. This avoids reimplementing SQLite→Postgres type
# translation in sed.
echo "  Creating accounts table manually..."
docker exec -i netbird-postgres psql -U netbird -d netbird <<'ACCT_SQL'
CREATE TABLE accounts (
  id                                           text PRIMARY KEY,
  created_by                                   text,
  created_at                                   timestamptz,
  domain                                       text,
  domain_category                              text,
  is_domain_primary_account                    numeric,
  network_identifier                           text,
  network_net                                  text,
  network_dns                                  text,
  network_serial                               bigint,
  dns_settings_disabled_management_groups      text,
  settings_peer_login_expiration_enabled       numeric,
  settings_peer_login_expiration               bigint,
  settings_peer_inactivity_expiration_enabled  numeric,
  settings_peer_inactivity_expiration          bigint,
  settings_regular_users_view_blocked          numeric,
  settings_groups_propagation_enabled          numeric,
  settings_jwt_groups_enabled                  numeric,
  settings_jwt_groups_claim_name               text,
  settings_jwt_allow_groups                    text,
  settings_routing_peer_dns_resolution_enabled numeric,
  settings_dns_domain                          text,
  settings_network_range                       text,
  settings_peer_expose_enabled                 numeric,
  settings_peer_expose_groups                  text,
  settings_extra_peer_approval_enabled         numeric,
  settings_extra_user_approval_required        numeric,
  settings_extra_integrated_validator          text,
  settings_extra_integrated_validator_groups   text,
  settings_lazy_connection_enabled             numeric default 0,
  settings_auto_update_version                 text default '"disabled"',
  reverse_proxy_free_domain_nonce              text
);
ACCT_SQL

# 5b: Import accounts data using sqlite3 .dump + grep INSERT
echo "  Importing accounts data..."
sqlite3 "$WORK_DIR/store.db" ".dump accounts" | grep "^INSERT" > /tmp/accounts_inserts.sql
if [ -s /tmp/accounts_inserts.sql ]; then
  docker exec -i netbird-postgres psql -U netbird -d netbird < /tmp/accounts_inserts.sql
  ACCT_COUNT=$(docker exec netbird-postgres psql -U netbird -d netbird -t -c "SELECT count(*) FROM accounts;" | tr -d ' ')
  echo "  accounts: $ACCT_COUNT row(s) imported."
else
  echo "  ERROR: No INSERT statements found for accounts."
  exit 1
fi

# 5c: pgloader for everything except accounts
echo "  Migrating remaining tables with pgloader..."
cat > /tmp/store.load << STOREEOF
LOAD DATABASE
  FROM sqlite://${WORK_DIR}/store.db
  INTO postgresql://netbird:${PG_PASS}@${PGHOST}:5432/netbird

EXCLUDING TABLE NAMES LIKE 'accounts'
;
STOREEOF
pgloader /tmp/store.load
echo "  store.db migrated."

# -----------------------------------------------
# Step 5b: Migrate idp.db (booleans already converted to integers)
# -----------------------------------------------
echo ""
echo "[5b/6] Migrating idp.db..."
if [ -f "$WORK_DIR/idp.db" ]; then
  pgloader --type sqlite "$WORK_DIR/idp.db" \
    "postgresql://netbird:${PG_PASS}@${PGHOST}:5432/netbird_idp"
  echo "  idp.db migrated."
else
  echo "  Skipped (file not found)."
fi

# -----------------------------------------------
# Step 6: Clean up orphaned records
# -----------------------------------------------
echo ""
echo "[6/6] Cleaning up orphaned records..."
docker exec netbird-postgres psql -U netbird -d netbird -c "
DELETE FROM policy_rules WHERE policy_id NOT IN (SELECT id FROM policies);
DELETE FROM routes WHERE account_id NOT IN (SELECT id FROM accounts);
DELETE FROM name_server_groups WHERE account_id NOT IN (SELECT id FROM accounts);
DROP TABLE IF EXISTS rules;
" 2>/dev/null || echo "  WARN: Some cleanup queries failed (tables may not exist — OK)."

echo ""
echo "============================================"
echo "  Migration complete"
echo "============================================"
echo ""
echo "  Verify row counts:"
docker exec netbird-postgres psql -U netbird -d netbird -c "SELECT 'peers' as tbl, count(*) FROM peers UNION ALL SELECT 'accounts', count(*) FROM accounts UNION ALL SELECT 'policies', count(*) FROM policies;"
docker exec netbird-postgres psql -U netbird -d netbird_events -c "SELECT count(*) as event_count FROM events;" 2>/dev/null || echo "  (no events table)"
echo ""
echo "  IMPORTANT: Verify accounts > 0. If 0, migration failed — do not proceed."
echo ""
echo "  Next: run switch-to-postgres.sh to reconfigure Netbird."
