#!/usr/bin/env bash
#
# ops/ansible/tests/local-smoke/smoke.sh
#
# Local docker-compose smoke test for the region A mgmt-host stack.
#
# PURPOSE ---------------------------------------------------------------------
# Catch the failure classes that have cost hours of SSM iteration on
# vega-bootstrap.yml. Runs the ENTIRE stack locally (no Ansible, no SSM, no
# cloud) and asserts the invariants that have bitten us:
#
#   1. pgbackrest.conf copied into postgres-a with docker cp -> a recreate
#      drops it and archive_command fails on every segment while Postgres
#      keeps them all (zagros). The base compose now bind-mounts
#      /etc/pgbackrest; this test recreates postgres-a and archives again.
#   2. host bind-mounts owned by the wrong uid -> pgbackrest stanza-create
#      fails rc=41 ("Permission denied" on archive.info).
#   3. pg_hba.conf missing 'host all postgres' line -> management-a
#      crash-loops with "no pg_hba.conf entry for user postgres".
#   4. DSN missing client_encoding=UTF8 -> management-a crash-loops in
#      migratePreAuto because the pgedge initdb defaults to SQL_ASCII.
#   5. postgres-a listens on 127.0.0.1 instead of 0.0.0.0 -> cross-host
#      Spock subs silently stay in 'down' state after container recreate.
#   6. signal-server listens on 127.0.0.1 -> federation unreachable and
#      peers enrolled at other sites can't coordinate.
#   7. PGDATA mounted at /var/lib/postgresql/data (Debian layout) instead
#      of /var/lib/pgsql/16/data (Red Hat / pgedge layout) -> container
#      silently runs on ephemeral intra-container storage.
#
# GROUND RULES ----------------------------------------------------------------
#   - Pure bash + docker + curl + grep. No Python, no Ansible.
#   - Read-only against the repo. A writable project dir under /tmp/smoke
#     holds the small number of files compose expects (management.json,
#     marker dir) and a compose OVERRIDE file that rebinds the pgBackRest
#     host paths to /tmp/smoke/... so the script works on a laptop without
#     sudo access to /var/log, /var/spool, /opt/docker-data.
#   - The override is a NEW file — the repo compose files are untouched.
#   - No cloud calls. Images are pulled from GHCR on first run.
#   - The base compose names no image for postgres-a or signal-a.
#     POSTGRES_SPOCK_IMAGE / SIGNAL_SERVER_IMAGE name them; unset, each is the
#     `fleet:` value of its row in ops/pins/fleet-pins.yaml, with
#     `{{ ghcr_org }}` taken from GHCR_ORG or ops/ansible/group_vars/all.yml.
#
# USAGE -----------------------------------------------------------------------
#   bash ops/ansible/tests/local-smoke/smoke.sh            # run + teardown
#   bash ops/ansible/tests/local-smoke/smoke.sh --keep     # leave stack up
#
# EXIT CODES ------------------------------------------------------------------
#   0  all assertions PASS
#   1  one or more FAIL; see summary table at the end
#   2  preflight failed (docker not running, port in use, not enough disk)

set -uo pipefail

# ============================================================================
# Config
# ============================================================================

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
COMPOSE_BASE="${REPO_ROOT}/ops/compose/mgmt-host/docker-compose.yml"
COMPOSE_OBS="${REPO_ROOT}/ops/compose/mgmt-host/docker-compose.obs.yml"
ADMIT_AUDIT_DIR="${REPO_ROOT}/ops/admit-audit"

SMOKE_ROOT="/tmp/smoke"
PROJECT_DIR="${SMOKE_ROOT}/mgmt-host"
OVERRIDE_COMPOSE="${PROJECT_DIR}/docker-compose.smoke-override.yml"

PROJECT_NAME="smoke-mgmt-host"
STANZA="smoke-netbird"

PIN_TABLE="${REPO_ROOT}/ops/pins/fleet-pins.yaml"
ALL_VARS="${REPO_ROOT}/ops/ansible/group_vars/all.yml"

# Ports the stack binds on the host. We refuse to start if any are in use
# because docker would silently fail bind and only one service would break.
REQUIRED_PORTS=(5432 10000 8080 9092 9093 9100 9124)

# Minimum headroom on both scratch and Docker's data filesystem. They can
# differ on Docker Desktop or with a tmpfs scratch directory. Image builds
# may need more; this is a refusal threshold, not a storage-size guarantee.
MIN_FREE_MB=4096

KEEP=0
if [[ "${1:-}" == "--keep" ]]; then
  KEEP=1
fi

# Results accumulator. Each check appends "PASS <name>" or "FAIL <name>: <msg>".
RESULTS=()

# ============================================================================
# Output helpers
# ============================================================================

log()    { printf '[smoke] %s\n' "$*"; }
header() { printf '\n===== %s =====\n' "$*"; }
pass()   { RESULTS+=("PASS $1"); printf '  PASS %s\n' "$1"; }
fail()   { RESULTS+=("FAIL $1: $2"); printf '  FAIL %s: %s\n' "$1" "$2"; }

die() {
  printf '[smoke] FATAL: %s\n' "$*" >&2
  exit 2
}

# ============================================================================
# Images
# ============================================================================

# pinned_image <ansible var>: the `fleet:` value of the pin-table row whose
# `var:` is the argument, with the registry organisation filled in. Prints
# nothing when the table, the row or the organisation cannot be read.
pinned_image() {
  local var="$1" ref org
  [[ -f "$PIN_TABLE" ]] || return 0
  ref="$(awk -v v="$var" '
    $1 == "var:" && $2 == v { hit = 1; next }
    hit && $1 == "fleet:"   { sub(/^[[:space:]]*fleet:[[:space:]]*/, ""); gsub(/"/, ""); print; exit }
    hit && /^  [^[:space:]#]/ { exit }
  ' "$PIN_TABLE")"
  [[ -n "$ref" && "$ref" != "null" ]] || return 0
  org="${GHCR_ORG:-}"
  if [[ -z "$org" && -f "$ALL_VARS" ]]; then
    org="$(sed -n "s/^ghcr_org:.* or '\\([^']*\\)'.*/\\1/p" "$ALL_VARS" | head -n 1)"
  fi
  [[ -n "$org" ]] || return 0
  printf '%s\n' "${ref//\{\{ ghcr_org \}\}/$org}"
}

resolve_images() {
  header "Images"
  : "${POSTGRES_SPOCK_IMAGE:=$(pinned_image postgres_spock_image)}"
  : "${SIGNAL_SERVER_IMAGE:=$(pinned_image signal_server_image)}"
  [[ -n "$POSTGRES_SPOCK_IMAGE" ]] || die "POSTGRES_SPOCK_IMAGE is unset and no postgres-spock pin could be read from $PIN_TABLE"
  [[ -n "$SIGNAL_SERVER_IMAGE"  ]] || die "SIGNAL_SERVER_IMAGE is unset and no signal-server pin could be read from $PIN_TABLE"
  log "postgres-a: $POSTGRES_SPOCK_IMAGE"
  log "signal-a:   $SIGNAL_SERVER_IMAGE"
}

# ============================================================================
# Preflight
# ============================================================================

preflight() {
  header "Preflight"

  command -v docker >/dev/null 2>&1 || die "docker not on PATH"
  command -v curl   >/dev/null 2>&1 || die "curl not on PATH"

  if ! docker info --format '{{.ServerVersion}}' >/dev/null 2>&1; then
    die "docker daemon not reachable (start Docker Desktop or dockerd)"
  fi
  log "docker daemon OK"

  # Disk headroom on the /tmp volume (POSIX df -k, then convert to MB).
  local free_kb
  free_kb="$(df -k /tmp | awk 'NR==2 {print $4}')"
  local free_mb=$(( free_kb / 1024 ))
  if (( free_mb < MIN_FREE_MB )); then
    die "only ${free_mb}MB free on /tmp, need >= ${MIN_FREE_MB}MB"
  fi
  log "disk free on /tmp: ${free_mb}MB"

  # Port conflict detection — macOS lsof + Linux ss fallback.
  local busy=()
  for p in "${REQUIRED_PORTS[@]}"; do
    if lsof -iTCP:"$p" -sTCP:LISTEN -nP 2>/dev/null | grep -q LISTEN; then
      busy+=("$p")
    elif command -v ss >/dev/null 2>&1 && ss -ltn "( sport = :$p )" 2>/dev/null | grep -q LISTEN; then
      busy+=("$p")
    fi
  done
  if (( ${#busy[@]} > 0 )); then
    die "ports already in use: ${busy[*]} — stop the owning processes first"
  fi
  log "ports free: ${REQUIRED_PORTS[*]}"

  # Required repo files.
  for f in "$COMPOSE_BASE" "$COMPOSE_OBS" "$ADMIT_AUDIT_DIR/Dockerfile"; do
    [[ -f "$f" ]] || die "missing repo file: $f"
  done
  log "repo compose files present"
}

# ============================================================================
# Project scaffolding
# ============================================================================
#
# We can't modify the repo compose files, but we ALSO can't use them verbatim:
# the base compose bind-mounts absolute host paths (/opt/docker-data/pgbackrest
# etc.) which don't exist on a laptop and would need sudo to create. The
# workaround is a SMALL override file in /tmp/smoke that rebinds those paths.
# The repo files stay untouched.

scaffold() {
  header "Scaffold project dir at ${PROJECT_DIR}"

  rm -rf "$SMOKE_ROOT"
  mkdir -p "$PROJECT_DIR/management-a/marker"
  mkdir -p "$SMOKE_ROOT/pgbackrest/data"
  mkdir -p "$SMOKE_ROOT/pgbackrest/log"
  mkdir -p "$SMOKE_ROOT/pgbackrest/spool"
  mkdir -p "$SMOKE_ROOT/pgbackrest/etc"

  # Render a minimal management.json. Upstream management refuses to start
  # without DataStoreEncryptionKey + Signal.URI. The test-only datastore key
  # must still decode to exactly 32 bytes. We don't test peer enrollment.
  cat > "$PROJECT_DIR/management-a/management.json" <<'JSON'
{
  "DataStoreEncryptionKey": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
  "Stuns": [],
  "TURNConfig": { "Turns": [] },
  "Signal": {
    "Proto": "http",
    "URI": "signal-a:10000"
  },
  "ReverseZone": "in-addr.arpa.",
  "Datadir": "/var/lib/netbird",
  "HttpConfig": {
    "Address": "0.0.0.0:80"
  },
  "StoreConfig": { "Engine": "postgres" },
  "IdpManagerConfig": { "ManagerType": "none" }
}
JSON

  # Override compose: rebinds the absolute pgbackrest host paths under
  # /tmp/smoke so this works on a laptop without sudo. DOES NOT modify the
  # repo compose — docker compose merges this on top via -f.
  cat > "$OVERRIDE_COMPOSE" <<YAML
# Smoke-test override. Rebinds pgbackrest bind-mounts to /tmp/smoke so the
# test works on a developer laptop without sudo. NOT a production file.
services:
  management-a:
    environment:
      # This fixture tests database startup, not geolocation. Avoid downloading
      # and importing external databases under the base service's memory cap.
      NB_DISABLE_GEOLOCATION: "true"
  postgres-a:
    volumes:
      - pgdata-a:/var/lib/pgsql/16/data
      - ${SMOKE_ROOT}/pgbackrest/data:/var/lib/pgbackrest
      - ${SMOKE_ROOT}/pgbackrest/log:/var/log/pgbackrest
      - ${SMOKE_ROOT}/pgbackrest/spool:/var/spool/pgbackrest
      - ${SMOKE_ROOT}/pgbackrest/etc:/etc/pgbackrest:ro
YAML

  # Dummy-but-valid env. The compose :?required checks enforce non-empty.
  cat > "$PROJECT_DIR/.env" <<ENV
POSTGRES_SUPERUSER_PASSWORD=smokepass
POSTGRES_SPOCK_IMAGE=${POSTGRES_SPOCK_IMAGE}
SIGNAL_SERVER_IMAGE=${SIGNAL_SERVER_IMAGE}
SITE_NAME=smoke-a
FEDERATION_PEERS=
PEER_NODE_EXPORTER_TARGET=127.0.0.1:9100
PEER_SIGNAL_METRICS_TARGET=127.0.0.1:9090
POSTGRES_EXPORTER_PASSWORD=smokeexp
ENV

  log "scaffold complete"
}

# ============================================================================
# Base stack
# ============================================================================

compose_base() {
  docker compose \
    --project-name "$PROJECT_NAME" \
    --project-directory "$PROJECT_DIR" \
    --env-file "$PROJECT_DIR/.env" \
    -f "$COMPOSE_BASE" \
    -f "$OVERRIDE_COMPOSE" \
    "$@"
}

docker_storage_preflight() {
  local image disk_output free_kb
  if ! image="$(compose_base config --images postgres-a)" \
      || [[ -z "$image" || "$image" == *$'\n'* ]]; then
    die "cannot resolve the postgres-a image for the Docker storage check"
  fi
  # An anonymous local volume uses the same daemon storage as the base PGDATA
  # volume, unlike a workstation's /tmp or the scratch bind mounts. Override
  # the entrypoint so this short-lived probe never starts a database.
  if ! disk_output="$(docker run --rm --network none --entrypoint df \
      --mount type=volume,dst=/var/lib/pgsql/16/data \
      "$image" -Pk /var/lib/pgsql/16/data)"; then
    die "cannot measure free space on Docker's data filesystem"
  fi
  free_kb="$(printf '%s\n' "$disk_output" | awk 'NR==2 {print $4}')"
  if [[ ! "$free_kb" =~ ^[0-9]+$ ]]; then
    die "Docker storage probe returned an invalid free-space value"
  fi
  if (( free_kb < MIN_FREE_MB * 1024 )); then
    die "Docker data filesystem has $((free_kb / 1024)) MB free; need at least ${MIN_FREE_MB} MB"
  fi
  pass "docker_data_space"
}

bring_up_base() {
  header "Bring up base stack (postgres-a + signal-a + management-a)"

  docker_storage_preflight

  # Fresh state — wipe any prior smoke run, volumes included.
  compose_base down -v --remove-orphans >/dev/null 2>&1 || true

  compose_base up -d postgres-a signal-a management-a || die "compose up failed"

  # Wait for postgres-a healthy (compose healthcheck is 10s intervals,
  # initdb takes ~20s on a cold image).
  log "waiting for postgres-a healthy (up to 2m)"
  local i
  for i in {1..60}; do
    local state
    state="$(docker inspect -f '{{.State.Health.Status}}' postgres-a 2>/dev/null || echo missing)"
    if [[ "$state" == "healthy" ]]; then
      log "postgres-a healthy after ${i}x2s"
      return 0
    fi
    sleep 2
  done
  docker logs --tail 50 postgres-a || true
  die "postgres-a did not reach healthy within 2m"
}

# ============================================================================
# Invariant checks
# ============================================================================

check_pg_hba_has_postgres() {
  # Regression: management-a crash-loops without this line.
  local name="pg_hba_has_postgres"
  if docker exec postgres-a grep -E '^host[[:space:]]+all[[:space:]]+postgres' \
       /var/lib/pgsql/16/data/pg_hba.conf >/dev/null 2>&1 \
     || docker exec postgres-a grep -E '^host[[:space:]]+all[[:space:]]+postgres' \
          /etc/postgresql/pg_hba.conf >/dev/null 2>&1; then
    pass "$name"
  else
    fail "$name" "no 'host all postgres' line in pg_hba.conf"
  fi
}

check_pgdata_persistent() {
  # Regression: wrong PGDATA path silently uses ephemeral storage.
  local name="pgdata_persistent"
  if docker exec postgres-a test -f /var/lib/pgsql/16/data/PG_VERSION; then
    pass "$name"
  else
    fail "$name" "PG_VERSION missing at /var/lib/pgsql/16/data (wrong PGDATA mount)"
  fi
}

check_signal_federation_port() {
  # Regression: 127.0.0.1:10000 breaks cross-site federation.
  local name="signal_federation_port"
  local port_bind
  port_bind="$(docker inspect signal-a \
    --format '{{range $p, $conf := .NetworkSettings.Ports}}{{range $conf}}{{$p}}={{.HostIp}}:{{.HostPort}};{{end}}{{end}}' 2>/dev/null || true)"
  if grep -q '10000/tcp=0.0.0.0:10000' <<<"$port_bind"; then
    pass "$name"
  else
    fail "$name" "signal-a :10000 not bound on 0.0.0.0 (got: ${port_bind})"
  fi
}

check_postgres_port_mesh() {
  # Regression: 127.0.0.1:5432 breaks cross-host Spock.
  local name="postgres_port_mesh"
  local port_bind
  port_bind="$(docker inspect postgres-a \
    --format '{{range $p, $conf := .NetworkSettings.Ports}}{{range $conf}}{{$p}}={{.HostIp}}:{{.HostPort}};{{end}}{{end}}' 2>/dev/null || true)"
  if grep -q '5432/tcp=0.0.0.0:5432' <<<"$port_bind"; then
    pass "$name"
  else
    fail "$name" "postgres-a :5432 not bound on 0.0.0.0 (got: ${port_bind})"
  fi
}

check_management_reachable() {
  # Not-connection-refused is enough: 200, 401, 404 all mean the HTTP server
  # is listening. We don't test auth.
  local name="management_reachable"
  # Give management-a time to run AutoMigrate on a cold DB.
  local i rc
  for i in {1..60}; do
    rc="$(curl -sf -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/ 2>/dev/null || true)"
    # Any HTTP response (even an error code) means the server is up. A
    # connection-refused / timeout leaves rc empty.
    if [[ -n "$rc" && "$rc" != "000" ]]; then
      pass "$name"
      return
    fi
    sleep 2
  done
  fail "$name" "management-a not reachable on 127.0.0.1:8080 after 2m"
}

check_dsn_encoding_works() {
  # Regression: DSN without client_encoding=UTF8 makes management-a
  # crash-loop in migratePreAuto on pgedge's SQL_ASCII initdb default.
  local name="dsn_encoding_works"
  local logs
  logs="$(docker logs management-a 2>&1 || true)"
  if grep -qi 'client_encoding' <<<"$logs" && grep -qi 'error' <<<"$logs"; then
    fail "$name" "management-a logs mention client_encoding error"
    return
  fi
  # Upstream management prints a "Started server" / "listen" / "HTTP server"
  # style line once migrations succeed. Match loosely — the exact wording
  # drifts across releases.
  if grep -Eiq 'listen|started|http server|server is running' <<<"$logs"; then
    pass "$name"
  else
    fail "$name" "no 'server listening' log line from management-a (startup stalled?)"
  fi
}

run_base_invariants() {
  header "Base-stack invariants"
  check_pg_hba_has_postgres
  check_pgdata_persistent
  check_signal_federation_port
  check_postgres_port_mesh
  check_management_reachable
  check_dsn_encoding_works
}

# ============================================================================
# pgBackRest smoke
# ============================================================================
#
# Mirrors ops/ansible/roles/pgbackrest/tasks/main.yml steps that bite during
# live runs: host dir perms, docker cp perm preservation, dnf install of the
# PGDG repo + pgbackrest, stanza-create.
#
# NOTE: this test intentionally does NOT bring up the pgbackrest-runner
# sidecar (docker-compose.pgbackrest.yml). Per vega-bootstrap Phase 5 §8 the
# PoC runs pgbackrest INSIDE postgres-a via docker exec — the sidecar is
# vestigial for the laptop test.

pgbackrest_smoke() {
  header "pgBackRest smoke (in-postgres-a)"

  # Host-side ownership mirrors the Ansible role: uid 26 (pgedge postgres
  # user) must own repo + log + spool so the container-side daemon can write.
  # macOS doesn't let us chown to uid 26 without sudo — skip the chown on
  # macOS and rely on Docker Desktop's permission relaxation. On Linux we
  # set it properly.
  if [[ "$(uname -s)" == "Linux" ]]; then
    sudo chown -R 26:26 \
      "$SMOKE_ROOT/pgbackrest/data" \
      "$SMOKE_ROOT/pgbackrest/log" \
      "$SMOKE_ROOT/pgbackrest/spool" 2>/dev/null || \
      log "chown to 26:26 failed (likely no sudo) — continuing"
  fi

  # Render a minimal pgbackrest.conf (LOCAL mode, no S3, no cipher — this is
  # a smoke test, not a production config).
  local conf="${SMOKE_ROOT}/pgbackrest/etc/pgbackrest.conf"
  cat > "$conf" <<CONF
[global]
repo1-path=/var/lib/pgbackrest
repo1-retention-full=1
log-path=/var/log/pgbackrest
log-level-console=info
log-level-file=detail

[${STANZA}]
pg1-path=/var/lib/pgsql/16/data
pg1-socket-path=/var/run/postgresql
pg1-port=5432
pg1-user=postgres
CONF

  # The base compose bind-mounts /etc/pgbackrest read-only (rebound to
  # ${SMOKE_ROOT}/pgbackrest/etc by the override). The role owns the host
  # file by uid 26, mode 0640; this smoke config holds no secret, so on
  # macOS (no chown to uid 26 without sudo) a world-readable 0644 stands in.
  chmod 0644 "$conf"
  if [[ "$(uname -s)" == "Linux" ]]; then
    if ! sudo sh -c "chown 26 '$conf' && chmod 0640 '$conf'" 2>/dev/null; then
      log "chown of pgbackrest.conf to uid 26 failed (likely no sudo); continuing"
    fi
  fi
  if docker exec -u postgres postgres-a test -r /etc/pgbackrest/pgbackrest.conf; then
    pass "pgbackrest_conf_readable"
  else
    fail "pgbackrest_conf_readable" "postgres cannot read /etc/pgbackrest/pgbackrest.conf through the bind mount"
    return 1
  fi

  # Install PGDG repo + pgbackrest. Wall-clock cost: ~30-60s on a warm image.
  # Skip if already installed (image may bake it in in a future version).
  if ! docker exec postgres-a sh -c 'command -v pgbackrest' >/dev/null 2>&1; then
    log "installing pgbackrest inside postgres-a (dnf from PGDG)"
    docker exec -u 0 postgres-a dnf install -y -q \
      "https://download.postgresql.org/pub/repos/yum/reporpms/EL-9-x86_64/pgdg-redhat-repo-latest.noarch.rpm" \
      >/tmp/smoke/dnf-repo.log 2>&1 || {
        fail "pgbackrest_install" "PGDG repo rpm install failed; see /tmp/smoke/dnf-repo.log"
        return 1
      }
    docker exec -u 0 postgres-a dnf install -y -q pgbackrest \
      >/tmp/smoke/dnf-pgbackrest.log 2>&1 || {
        fail "pgbackrest_install" "pgbackrest install failed; see /tmp/smoke/dnf-pgbackrest.log"
        return 1
      }
  fi
  pass "pgbackrest_install"

  # archive_mode + archive_command, then restart (required for archive_mode).
  docker exec -e PGPASSWORD=smokepass postgres-a \
    psql -U postgres -v ON_ERROR_STOP=1 \
      -c "ALTER SYSTEM SET archive_mode='on';" \
      -c "ALTER SYSTEM SET archive_command='pgbackrest --config=/etc/pgbackrest/pgbackrest.conf --stanza=${STANZA} archive-push %p';" \
      -c "ALTER SYSTEM SET archive_timeout='60s';" \
      >/dev/null
  compose_base restart postgres-a >/dev/null 2>&1
  for i in {1..30}; do
    docker exec postgres-a pg_isready -U postgres >/dev/null 2>&1 && break
    sleep 2
  done

  # stanza-create: the actual payoff. This is where bind-mount perm bugs
  # manifest as "Permission denied" on archive.info.
  local sc_out sc_rc
  sc_out="$(docker exec postgres-a pgbackrest --stanza="$STANZA" stanza-create 2>&1)"
  sc_rc=$?
  if (( sc_rc == 0 )); then
    pass "pgbackrest_stanza_create"
  else
    fail "pgbackrest_stanza_create" "rc=${sc_rc}: ${sc_out}"
    return 1
  fi

  # archive.info is the artifact we actually care about persisting.
  if docker exec postgres-a test -f "/var/lib/pgbackrest/archive/${STANZA}/archive.info"; then
    pass "pgbackrest_archive_info"
  else
    fail "pgbackrest_archive_info" "archive.info not created"
    return 1
  fi

  # A recreate must not stop archiving. archive_command lives in PGDATA and
  # survives; the config it reads has to survive too. `pgbackrest check`
  # switches WAL and waits for that segment to reach the repository.
  local before_id after_id ready=0
  if ! before_id="$(docker inspect --format '{{.Id}}' postgres-a)" || [[ -z "$before_id" ]]; then
    fail "pgbackrest_archive_survives_recreate" "cannot identify the original postgres-a container"
    return 1
  fi
  if ! compose_base up -d --force-recreate --no-deps postgres-a >/dev/null 2>&1; then
    fail "pgbackrest_archive_survives_recreate" "postgres-a recreation failed"
    return 1
  fi
  if ! after_id="$(docker inspect --format '{{.Id}}' postgres-a)" \
      || [[ -z "$after_id" || "$after_id" == "$before_id" ]]; then
    fail "pgbackrest_archive_survives_recreate" "recreation did not produce a different postgres-a container"
    return 1
  fi
  for i in {1..30}; do
    if docker exec postgres-a pg_isready -U postgres >/dev/null 2>&1; then ready=1; break; fi
    sleep 2
  done
  if (( ready == 0 )); then
    fail "pgbackrest_archive_survives_recreate" "recreated postgres-a did not become ready"
    return 1
  fi
  local ck_out ck_rc
  ck_out="$(docker exec postgres-a pgbackrest --stanza="$STANZA" check 2>&1)"
  ck_rc=$?
  if (( ck_rc == 0 )); then
    pass "pgbackrest_archive_survives_recreate"
  else
    fail "pgbackrest_archive_survives_recreate" "rc=${ck_rc} after recreating postgres-a: ${ck_out}"
    return 1
  fi
}

# ============================================================================
# admit-audit smoke
# ============================================================================
#
# Builds the image (this is one of the things CI verifies) and runs the
# container against postgres-a. Checks that :9124/metrics returns 200.

admit_audit_smoke() {
  header "admit-audit smoke"

  local img="smoke/admit-audit:local"
  local build_start build_end
  build_start=$(date +%s)

  log "building admit-audit image from ${ADMIT_AUDIT_DIR} (cap 5m)"
  if ! docker build -t "$img" "$ADMIT_AUDIT_DIR" >/tmp/smoke/admit-audit-build.log 2>&1; then
    fail "admit_audit_build" "docker build failed; see /tmp/smoke/admit-audit-build.log"
    return
  fi
  build_end=$(date +%s)
  local build_secs=$(( build_end - build_start ))
  if (( build_secs > 300 )); then
    fail "admit_audit_build" "build took ${build_secs}s (cap 300s)"
  else
    pass "admit_audit_build"
    log "admit-audit build: ${build_secs}s"
  fi

  # Create the read-only role the sidecar connects as. Mirrors what the
  # admit-audit Ansible role does before `docker compose up` is run with
  # the overlay. netbird DB may not exist yet on cold-start — create if
  # missing.
  docker exec -e PGPASSWORD=smokepass postgres-a \
    psql -U postgres -v ON_ERROR_STOP=1 -c \
      "SELECT 1 FROM pg_database WHERE datname='netbird';" \
      | grep -q '1 row' \
    || docker exec -e PGPASSWORD=smokepass postgres-a \
         psql -U postgres -v ON_ERROR_STOP=1 -c "CREATE DATABASE netbird;" >/dev/null

  docker exec -e PGPASSWORD=smokepass postgres-a \
    psql -U postgres netbird -v ON_ERROR_STOP=1 -c \
      "DO \$\$ BEGIN
         IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='admit_audit') THEN
           CREATE ROLE admit_audit LOGIN PASSWORD 'smokeaudit';
         END IF;
       END \$\$;" >/dev/null 2>&1 || true
  docker exec -e PGPASSWORD=smokepass postgres-a \
    psql -U postgres netbird -c \
      "CREATE TABLE IF NOT EXISTS peers (id text PRIMARY KEY);
       GRANT SELECT ON peers TO admit_audit;" >/dev/null 2>&1 || true

  # Run the sidecar attached to the same compose network so it can resolve
  # postgres-a by name.
  docker rm -f admit-audit-smoke >/dev/null 2>&1 || true
  docker run -d --rm \
    --name admit-audit-smoke \
    --network mgmt \
    -e ADMIT_AUDIT_DSN="postgres://admit_audit:smokeaudit@postgres-a:5432/netbird?sslmode=disable" \
    -e SITE_NAME=smoke-a \
    -p 127.0.0.1:9124:9124 \
    "$img" \
    --listen=0.0.0.0:9124 --poll=30s --window=5m \
    >/dev/null || {
      fail "admit_audit_run" "docker run failed"
      return
    }

  # Metrics endpoint — first scrape.
  local i
  for i in {1..15}; do
    if curl -sf -o /dev/null http://127.0.0.1:9124/metrics 2>/dev/null; then
      pass "admit_audit_metrics"
      return
    fi
    sleep 2
  done
  fail "admit_audit_metrics" "no 200 from :9124/metrics within 30s"
  docker logs --tail 30 admit-audit-smoke || true
}

# ============================================================================
# Teardown
# ============================================================================

teardown() {
  if (( KEEP == 1 )); then
    log "--keep set, leaving stack + /tmp/smoke in place"
    log "tear down manually with:"
    log "  docker compose -p ${PROJECT_NAME} -f ${COMPOSE_BASE} -f ${OVERRIDE_COMPOSE} down -v"
    log "  docker rm -f admit-audit-smoke; docker rmi smoke/admit-audit:local; rm -rf ${SMOKE_ROOT}"
    return
  fi

  header "Teardown"
  docker rm -f admit-audit-smoke >/dev/null 2>&1 || true
  compose_base down -v --remove-orphans >/dev/null 2>&1 || true
  docker rmi smoke/admit-audit:local >/dev/null 2>&1 || true
  rm -rf "$SMOKE_ROOT"
  log "torn down"
}

# ============================================================================
# Summary
# ============================================================================

summary() {
  header "Summary"
  local pass_count=0 fail_count=0 line
  for line in "${RESULTS[@]}"; do
    printf '  %s\n' "$line"
    case "$line" in
      PASS*) pass_count=$(( pass_count + 1 )) ;;
      FAIL*) fail_count=$(( fail_count + 1 )) ;;
    esac
  done
  printf '\n  %d pass, %d fail\n' "$pass_count" "$fail_count"
  if (( fail_count > 0 )); then
    return 1
  fi
  return 0
}

# ============================================================================
# Main
# ============================================================================

main() {
  preflight
  resolve_images
  scaffold
  bring_up_base
  run_base_invariants
  pgbackrest_smoke
  admit_audit_smoke

  # Teardown runs regardless of invariant outcome (unless --keep).
  teardown

  summary
}

main "$@"
