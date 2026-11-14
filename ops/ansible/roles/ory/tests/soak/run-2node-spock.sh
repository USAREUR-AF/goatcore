#!/usr/bin/env bash
#
# ╔════════════════════════════════════════════════════════════════════════╗
# ║ RUNS ON: a Linux EC2 host — DO NOT run locally on macOS.               ║
# ║                                                                        ║
# ║ Why: F-008 (docs/findings/F-008-docker-exec-psql-hang-under-load.md). ║
# ║      macOS Docker Desktop unrecoverably wedges under bidirectional     ║
# ║      postgres-spock + sustained writes + high-frequency `docker exec`. ║
# ║      Recovery takes a FULL SYSTEM RESTART (Docker Desktop restart      ║
# ║      insufficient).                                                    ║
# ║                                                                        ║
# ║ Run via SSM exec on the Linux host.                                    ║
# ╚════════════════════════════════════════════════════════════════════════╝
#
# Phase 2+ 2-node Spock-replicated Kratos+Hydra soak.
#
# ⚠️ FRAMING CORRECTION: an earlier comment in this file
# described this as testing "intra-site replication of identity state for
# keepalived A↔B failover." That was wrong. Intra-site (within one LAN)
# uses plain PostgreSQL streaming replication + VRRP failover — see
# docs/design/ha-failover.md. Spock multi-master is for INTER-SITE
# active-active between sites — see
# docs/design/ha-multi-site.md. What this driver actually exercises is
# the SHAPE of inter-site Spock replication: two postgres-spock instances
# doing bidirectional logical replication with sustained writes, run on
# one host for convenience.
#
# ⚠️ PATTERN-C / D8 AMENDMENT: this driver uses the WIDE
# repset model — `repset_add_all_tables('default', 'public')` replicating
# every kratos + hydra public table. ADR 1010 D8
# supersedes that scope for any future production activation, which
# uses the NARROW `ory_narrow` repset (10 tables only — identity rows
# + signing keys + nid scaffolding; see
# docs/project/ory-spock-narrow-repset-schema-audit.md).
#
# A14 (Kratos flow A→B) and A15 (Hydra token A→B introspect) are this
# driver's existing PASS gates. Under D8 those properties correctly do
# NOT hold (flows + tokens are exclusion-list, stay site-local). The
# Pattern C plan Sub-block E retires A14 + A15 and introduces:
#   A14'  identity row replicates A→B within Spock lag budget
#   A15'  Hydra signing key replicates A→B (via deliberate JWK rotation)
#   A17   auth code issued at A is NOT consumable at B (single-use
#         preserved by exclusion-list correctness)
#   A18   JWT issued at A validates at B via JWKS (DB replication
#         not required for cross-site auth)
#
# This driver remains useful as the wide-repset reference fixture for
# ADR 1010 trigger-condition revisits. The narrow-repset replacement
# driver is authored against a live fixture during the
# operator-fired Sub-block D pilot activation. Until then, A14 + A15
# PASSing here are SOAK-HARNESS measurements that do NOT correspond
# to production behavior — production runs nothing yet (the Spock
# extension is not even installed in kratos + hydra DBs on any
# production site).
#
# Findings exercised by this driver (per ADR 1011 register; see
# docs/findings/INDEX.md):
#   F-001 Kratos networks nid multi-tenancy   (ADR 1009)
#   F-002 Hydra hydra_client.nid analog       (ADR 1009)
#   F-003 Spock 5 LWW per-row default         (ADR 1010 D4+D5)
#   F-004 repset_add_all_tables no-PK reject  (workaround in this driver)
#   F-005 Kratos cipher secret per-instance   (deliberately exercised here)
#   F-006 postgres-spock init post-init bounce (wait_pg workaround below)
#
# Purpose: validate Assumptions A3, A4, A8, A9 of the Ory decision review
# (docs/project/idp-decision-review.md) — that Kratos and Hydra schemas
# replicate cleanly across two postgres-spock nodes via Spock multi-master
# logical replication, the way netbird's `netbird` + `netbird_idp` databases
# already do (ADR 0411 + ADR 0401, per-site sovereign). This is INTER-SITE
# Spock validation — intra-site Router-A↔Router-B sync uses plain Postgres
# streaming replication + VRRP, NOT Spock (see ha-failover.md).
#
# Topology:
#   ┌──────────────────────────┐         ┌──────────────────────────┐
#   │ postgres-a (15435/host)  │ ←Spock→ │ postgres-b (15436/host)  │
#   │   db: kratos, hydra      │   bidir │   db: kratos, hydra      │
#   └──────────────────────────┘         └──────────────────────────┘
#       ↑                                    ↑
#   kratos-a (24433/24434)               kratos-b (24533/24534)
#   hydra-a  (24444/24445)               hydra-b  (24544/24545)
#
# What's exercised (additive to single-node soak):
#   - Spock node_create + sub_create both directions for kratos + hydra
#   - Default repset add_all_tables(*) on each side after migration
#   - Identity-create at A — verify present at B (lag observed in JSONL)
#   - Client-create at B — verify present at A
#   - Token-grant at A — verify introspect_ok at B (cross-node token usage)
#   - F3: concurrent same-email identity-create at A and B, with separate
#     HTTP statuses and identity IDs. Exactly one 201 plus one 409 establishes
#     an observed rejection; two 201 responses expose concurrent acceptance.
#     Admin-API-only writes do not establish uniqueness across database nodes.
#
# What's NOT exercised here (separate Phase 2 tests):
#   - Webauthn / passkey real browser flow (A10) — manual hands-on
#
# Failure injection (opt-in, gated by $SOAK_NEMESIS):
#   When $SOAK_NEMESIS=1, every $NEMESIS_INTERVAL ticks the driver fires
#   one of two nemesis events, alternating each fire:
#     - "pause_postgres_b" — `docker pause postgres-b` for $NEMESIS_HOLD_S
#       seconds, then unpause. Stalls all writes to B and Spock replay
#       on B without changing the network plumbing. Validates ADR 1010
#       D2 (lifespan budget exceeds the realized partition window) and
#       D5 (ACE drift-watch surfaces residual divergence) on the tick
#       and the one immediately after.
#     - "kill_kratos_a" — `docker kill kratos-a`; the in-process
#       mgmt-host-compose-watch (F-013 mitigation, started after the
#       four-daemon ready-check below) revives kratos-a within ≤15s on
#       its next poll cycle. Validates F-013 closure under load AND
#       ADR 1010 D3 (service-side bounded retry on transient 404/410):
#       Kratos reconnects to its DSN with no operator action.
# Docker's `restart: unless-stopped` does not trigger on `docker kill`
# on 29.1.3, so without the watcher kratos-a stays dead.
#   Subset of ADR 1010 D6 F1 + F2 — the full F1/F2 cases also need a
#   real VIP-flip orchestration, which is outside this driver's scope.
#   Off by default; enable for canonical (72 h) tier runs. Other knobs:
#     $NEMESIS_INTERVAL (default 20 ticks ≈ 10 min)
#     $NEMESIS_HOLD_S   (default 15 s — pause window for pause_postgres_b)
#
# Default duration: 2 hours. Override with $SOAK_DURATION.
#
# Output: appends JSON lines per tick to
#   docs/reports/<TODAY>-ory-spock-replicated-soak-evidence.jsonl
#
# Run (steady-state):
#   ops/ansible/roles/ory/tests/soak/run-2node-spock.sh
#
# Run (canonical tier with nemesis):
#   SOAK_DURATION=72h SOAK_NEMESIS=1 \
#     ops/ansible/roles/ory/tests/soak/run-2node-spock.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../../../../../.." && pwd)"
SOAK_DURATION="${SOAK_DURATION:-2h}"
TODAY="$(date -u +%Y-%m-%d)"
# EVIDENCE_FILE override: when the script runs from a non-repo location
# (e.g., pushed to /tmp on a test host), REPO_ROOT computes to "/" and the
# default below resolves to /docs/reports/... which silently fails to
# write because /docs doesn't exist. Wrappers should pass an explicit
# absolute path. Always mkdir -p the parent before writing.
EVIDENCE_FILE="${EVIDENCE_FILE:-$REPO_ROOT/docs/reports/${TODAY}-ory-spock-replicated-soak-evidence.jsonl}"
mkdir -p "$(dirname "$EVIDENCE_FILE")" || { echo "FATAL: cannot create evidence dir for $EVIDENCE_FILE"; exit 1; }
TICK_INTERVAL=30

# Failure injection (off by default; canonical-tier soaks set SOAK_NEMESIS=1).
# When enabled, nemesis events fire every NEMESIS_INTERVAL ticks, alternating
# between pause_postgres_b and kill_kratos_a. NEMESIS_HOLD_S is the
# pause_postgres_b window (kill_kratos_a is a single signal — recovery time
# is observed by the next tick).
SOAK_NEMESIS="${SOAK_NEMESIS:-0}"
NEMESIS_INTERVAL="${NEMESIS_INTERVAL:-20}"
NEMESIS_HOLD_S="${NEMESIS_HOLD_S:-15}"

WORK="$(mktemp -d -t ory-2node-soak-XXXXXX)"
RUN_ID="$(basename "$WORK" | tr -d 'a-z-')"
[ -z "$RUN_ID" ] && RUN_ID="$$"
WATCHER_PID=""
trap 'if [ -n "${WATCHER_PID:-}" ]; then kill -TERM "$WATCHER_PID" 2>/dev/null || true; wait "$WATCHER_PID" 2>/dev/null || true; fi; docker compose -f "$WORK/docker-compose.yml" down -v >/dev/null 2>&1 || true; rm -rf "$WORK"' EXIT

# Fixed test secrets — fixture only.
export POSTGRES_SUPERUSER_PASSWORD="ory2node-pg-fixed-aaaa-bbbb-cccc"
export ORY_KRATOS_IMAGE="oryd/kratos:v1.3.1"
export ORY_HYDRA_IMAGE="oryd/hydra:v2.3.0"
export HYDRA_SECRETS_SYSTEM="ory2node-hydra-system-secret-fixed32"
export HYDRA_SECRETS_COOKIE="ory2node-hydra-cookie-secret-fixed32"
# Kratos secrets — docker-compose.ory.yml hard-requires
# ${KRATOS_SECRETS_COOKIE:?...} and ${KRATOS_SECRETS_CIPHER:?...}.
# CIPHER must be EXACTLY 32 bytes (Kratos enforces; AES-256 key).
export KRATOS_SECRETS_COOKIE="ory2node-kratos-cookie-secret-fxd32"
export KRATOS_SECRETS_CIPHER="ory2nodekratoscipher32bytesxxxxx"

# Host-side ports — chosen high to coexist with run-single-node.sh (24xxx).
PG_A_PORT=25435; PG_B_PORT=25436
KRATOS_A_PUB=25433; KRATOS_A_ADM=25434
HYDRA_A_PUB=25444;  HYDRA_A_ADM=25445
KRATOS_B_PUB=25533; KRATOS_B_ADM=25534
HYDRA_B_PUB=25544;  HYDRA_B_ADM=25545

mkdir -p "$WORK/ory"

# Identity schema — same as fixture.
cat > "$WORK/ory/identity-schema.json" <<'EOF'
{"$id":"local","$schema":"http://json-schema.org/draft-07/schema#","title":"X","type":"object","properties":{"traits":{"type":"object","properties":{"email":{"type":"string","format":"email","ory.sh/kratos":{"credentials":{"password":{"identifier":true}}}},"name":{"type":"string"}},"required":["email"],"additionalProperties":false}}}
EOF

# Kratos config templates — one per node so issuer/base URL is correct.
write_kratos_config() {
  local node="$1" pub="$2"
  cat > "$WORK/ory/kratos-${node}.yaml" <<EOF
version: v1.3.1
serve:
  public:
    base_url: "http://127.0.0.1:${pub}/"
    cors:
      enabled: true
  admin:
    base_url: http://kratos-${node}:4434/
identity:
  default_schema_id: default
  schemas:
    - id: default
      url: file:///etc/config/identity-schema.json
selfservice:
  default_browser_return_url: "http://127.0.0.1:${pub}/"
  flows:
    settings: { ui_url: "http://127.0.0.1:${pub}/settings" }
    login:    { ui_url: "http://127.0.0.1:${pub}/login" }
    registration: { ui_url: "http://127.0.0.1:${pub}/register" }
secrets:
  cookie:
    - "ory2node-cookie-secret-${node}-do-not-use"
  cipher:
    # Kratos requires cipher secret length == 32 exactly. Per-node value
    # so each Kratos instance has its own (Spock replicates the data they
    # encrypt, so they need to share the cipher key — but for this fixture
    # the differing keys exercise the worst-case "no shared cipher" path.
    # Production would distribute one cipher key per HA pair via a
    # fetched secret manifest.)
    - "ory2node-${node}-cipher-32chrs-aaaaaaa"
hashers:
  algorithm: bcrypt
  bcrypt: { cost: 8 }
courier:
  smtp:
    connection_uri: "smtps://localhost:1025/?disable_starttls=true"
session:
  lifespan: 24h
EOF
}
write_hydra_config() {
  local node="$1" pub="$2"
  cat > "$WORK/ory/hydra-${node}.yaml" <<EOF
serve:
  public: { port: 4444 }
  admin:  { port: 4445 }
urls:
  self:
    issuer: "http://127.0.0.1:${pub}/"
  consent: "http://127.0.0.1:3000/consent"
  login:   "http://127.0.0.1:3000/login"
secrets:
  system:
    - "${HYDRA_SECRETS_SYSTEM}"
oidc:
  subject_identifiers:
    supported_types: [public]
oauth2:
  expose_internal_errors: true
EOF
}
write_kratos_config a "$KRATOS_A_PUB"
write_kratos_config b "$KRATOS_B_PUB"
write_hydra_config  a "$HYDRA_A_PUB"
write_hydra_config  b "$HYDRA_B_PUB"

# Compose: two postgres-spock + two kratos + two hydra + their migrate
# one-shots. All on the same docker network so postgres-a <-> postgres-b
# can speak Spock replication.
cat > "$WORK/docker-compose.yml" <<EOF
services:
  postgres-a:
    image: ${POSTGRES_SPOCK_IMAGE:-postgres-spock-test:contribs}
    command: ["postgres", "-c", "config_file=/etc/postgresql/postgresql.conf"]
    environment:
      POSTGRES_PASSWORD: \${POSTGRES_SUPERUSER_PASSWORD}
    ports: ["127.0.0.1:${PG_A_PORT}:5432"]
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 2s
      retries: 30
    networks: [mgmt]

  postgres-b:
    image: ${POSTGRES_SPOCK_IMAGE:-postgres-spock-test:contribs}
    command: ["postgres", "-c", "config_file=/etc/postgresql/postgresql.conf"]
    environment:
      POSTGRES_PASSWORD: \${POSTGRES_SUPERUSER_PASSWORD}
    ports: ["127.0.0.1:${PG_B_PORT}:5432"]
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 2s
      retries: 30
    networks: [mgmt]

  kratos-migrate-a:
    image: \${ORY_KRATOS_IMAGE}
    depends_on: { postgres-a: { condition: service_healthy } }
    environment:
      DSN: "postgres://postgres:\${POSTGRES_SUPERUSER_PASSWORD}@postgres-a:5432/kratos?sslmode=disable"
    command: ["migrate", "-c", "/etc/config/kratos.yaml", "sql", "-e", "--yes"]
    volumes:
      - ./ory/kratos-a.yaml:/etc/config/kratos.yaml:ro
      - ./ory/identity-schema.json:/etc/config/identity-schema.json:ro
    restart: "no"
    networks: [mgmt]

  kratos-migrate-b:
    image: \${ORY_KRATOS_IMAGE}
    depends_on: { postgres-b: { condition: service_healthy } }
    environment:
      DSN: "postgres://postgres:\${POSTGRES_SUPERUSER_PASSWORD}@postgres-b:5432/kratos?sslmode=disable"
    command: ["migrate", "-c", "/etc/config/kratos.yaml", "sql", "-e", "--yes"]
    volumes:
      - ./ory/kratos-b.yaml:/etc/config/kratos.yaml:ro
      - ./ory/identity-schema.json:/etc/config/identity-schema.json:ro
    restart: "no"
    networks: [mgmt]

  hydra-migrate-a:
    image: \${ORY_HYDRA_IMAGE}
    depends_on: { postgres-a: { condition: service_healthy } }
    environment:
      DSN: "postgres://postgres:\${POSTGRES_SUPERUSER_PASSWORD}@postgres-a:5432/hydra?sslmode=disable"
    command: ["migrate", "-c", "/etc/config/hydra.yaml", "sql", "-e", "--yes"]
    volumes: [ "./ory/hydra-a.yaml:/etc/config/hydra.yaml:ro" ]
    restart: "no"
    networks: [mgmt]

  hydra-migrate-b:
    image: \${ORY_HYDRA_IMAGE}
    depends_on: { postgres-b: { condition: service_healthy } }
    environment:
      DSN: "postgres://postgres:\${POSTGRES_SUPERUSER_PASSWORD}@postgres-b:5432/hydra?sslmode=disable"
    command: ["migrate", "-c", "/etc/config/hydra.yaml", "sql", "-e", "--yes"]
    volumes: [ "./ory/hydra-b.yaml:/etc/config/hydra.yaml:ro" ]
    restart: "no"
    networks: [mgmt]

  kratos-a:
    image: \${ORY_KRATOS_IMAGE}
    depends_on:
      kratos-migrate-a: { condition: service_completed_successfully }
    environment:
      DSN: "postgres://postgres:\${POSTGRES_SUPERUSER_PASSWORD}@postgres-a:5432/kratos?sslmode=disable"
    ports:
      - "127.0.0.1:${KRATOS_A_PUB}:4433"
      - "127.0.0.1:${KRATOS_A_ADM}:4434"
    command: ["serve", "-c", "/etc/config/kratos.yaml", "--dev", "--watch-courier"]
    volumes:
      - ./ory/kratos-a.yaml:/etc/config/kratos.yaml:ro
      - ./ory/identity-schema.json:/etc/config/identity-schema.json:ro
    networks: [mgmt]
    restart: unless-stopped  # Docker 29.1.3 doesn't trigger 'unless-stopped' recovery on `docker kill`; the soak driver starts mgmt-host-compose-watch in-process as the external supervisor.

  kratos-b:
    image: \${ORY_KRATOS_IMAGE}
    depends_on:
      kratos-migrate-b: { condition: service_completed_successfully }
    environment:
      DSN: "postgres://postgres:\${POSTGRES_SUPERUSER_PASSWORD}@postgres-b:5432/kratos?sslmode=disable"
    ports:
      - "127.0.0.1:${KRATOS_B_PUB}:4433"
      - "127.0.0.1:${KRATOS_B_ADM}:4434"
    command: ["serve", "-c", "/etc/config/kratos.yaml", "--dev", "--watch-courier"]
    volumes:
      - ./ory/kratos-b.yaml:/etc/config/kratos.yaml:ro
      - ./ory/identity-schema.json:/etc/config/identity-schema.json:ro
    networks: [mgmt]
    restart: unless-stopped  # Docker 29.1.3 doesn't trigger 'unless-stopped' recovery on `docker kill`; the soak driver starts mgmt-host-compose-watch in-process as the external supervisor.

  hydra-a:
    image: \${ORY_HYDRA_IMAGE}
    depends_on:
      hydra-migrate-a: { condition: service_completed_successfully }
    environment:
      DSN: "postgres://postgres:\${POSTGRES_SUPERUSER_PASSWORD}@postgres-a:5432/hydra?sslmode=disable"
      SECRETS_SYSTEM: "\${HYDRA_SECRETS_SYSTEM}"
      SECRETS_COOKIE: "\${HYDRA_SECRETS_COOKIE}"
    ports:
      - "127.0.0.1:${HYDRA_A_PUB}:4444"
      - "127.0.0.1:${HYDRA_A_ADM}:4445"
    command: ["serve", "-c", "/etc/config/hydra.yaml", "all", "--dev"]
    volumes: [ "./ory/hydra-a.yaml:/etc/config/hydra.yaml:ro" ]
    networks: [mgmt]
    restart: unless-stopped  # Docker 29.1.3 doesn't trigger 'unless-stopped' recovery on `docker kill`; the soak driver starts mgmt-host-compose-watch in-process as the external supervisor.

  hydra-b:
    image: \${ORY_HYDRA_IMAGE}
    depends_on:
      hydra-migrate-b: { condition: service_completed_successfully }
    environment:
      DSN: "postgres://postgres:\${POSTGRES_SUPERUSER_PASSWORD}@postgres-b:5432/hydra?sslmode=disable"
      SECRETS_SYSTEM: "\${HYDRA_SECRETS_SYSTEM}"
      SECRETS_COOKIE: "\${HYDRA_SECRETS_COOKIE}"
    ports:
      - "127.0.0.1:${HYDRA_B_PUB}:4444"
      - "127.0.0.1:${HYDRA_B_ADM}:4445"
    command: ["serve", "-c", "/etc/config/hydra.yaml", "all", "--dev"]
    volumes: [ "./ory/hydra-b.yaml:/etc/config/hydra.yaml:ro" ]
    networks: [mgmt]
    restart: unless-stopped  # Docker 29.1.3 doesn't trigger 'unless-stopped' recovery on `docker kill`; the soak driver starts mgmt-host-compose-watch in-process as the external supervisor.

networks:
  mgmt:
    name: mgmt-ory2node-${RUN_ID}
EOF

cd "$WORK"

echo "[2node-soak] pull images"
docker compose pull --ignore-pull-failures >/dev/null 2>&1 || true

echo "[2node-soak] up postgres-a + postgres-b"
docker compose up -d postgres-a postgres-b
for _ in $(seq 1 60); do
  HA=$(docker inspect -f '{{.State.Health.Status}}' "$(docker compose ps -q postgres-a)" 2>/dev/null || echo)
  HB=$(docker inspect -f '{{.State.Health.Status}}' "$(docker compose ps -q postgres-b)" 2>/dev/null || echo)
  [ "$HA" = "healthy" ] && [ "$HB" = "healthy" ] && break
  sleep 1
done

PG_A=$(docker compose ps -q postgres-a)
PG_B=$(docker compose ps -q postgres-b)

# postgres-spock images run init scripts on first start which trigger a
# post-init restart. Wait until psql can run a trivial query inside each
# container — that's the real ready signal, not just `pg_isready`.
# Ready means the FINAL server answers. During first-boot init the entrypoint
# runs a temporary server on the Unix socket only, then shuts it down, so a
# socket probe can succeed against the server that is about to stop ("the
# database system is shutting down" on the next query). Probe over TCP, which
# only the final server listens on, and require three answers in a row.
wait_pg() {
  local cid="$1" ok=0
  for _ in $(seq 1 120); do
    if docker exec -e PGPASSWORD="$POSTGRES_SUPERUSER_PASSWORD" "$cid" \
         psql -h 127.0.0.1 -U postgres -d postgres -tAXc "SELECT 1" >/dev/null 2>&1; then
      ok=$((ok + 1)); [ "$ok" -ge 3 ] && return 0
    else
      ok=0
    fi
    sleep 2
  done
  return 1
}
echo "[2node-soak] waiting for postgres-a + postgres-b post-init readiness"
wait_pg "$PG_A" || { echo "FATAL: postgres-a never became query-ready"; exit 1; }
wait_pg "$PG_B" || { echo "FATAL: postgres-b never became query-ready"; exit 1; }

# Helper to run psql in either container.
# Wrap docker-exec-psql in `timeout` because Docker Desktop on macOS
# (and probably any Docker daemon under sustained load) can let
# `docker exec` hang on what should be a non-blocking system-view
# query, and one hung query blocks every subsequent tick. This wrapper is the workaround.
# 8 s is generous for any single query/exec round-trip in this fixture.
PSQL_TIMEOUT="${PSQL_TIMEOUT:-8}"
psqlA() { timeout "${PSQL_TIMEOUT}s" docker exec -e PGPASSWORD="$POSTGRES_SUPERUSER_PASSWORD" "$PG_A" psql -U postgres -v ON_ERROR_STOP=1 "$@"; }
psqlB() { timeout "${PSQL_TIMEOUT}s" docker exec -e PGPASSWORD="$POSTGRES_SUPERUSER_PASSWORD" "$PG_B" psql -U postgres -v ON_ERROR_STOP=1 "$@"; }

echo "[2node-soak] create kratos+hydra DBs on both nodes + install spock+contribs"
# postgres-spock's own init script already creates kratos+hydra and installs
# pg_trgm + btree_gin (ops/postgres/spock/init/02-spock-extension.sql), so
# CREATE DATABASE / CREATE EXTENSION is mostly redundant — but spock is NOT
# installed for kratos/hydra by the init script (intentionally; it's a
# Phase 2+ step). Tolerate "already exists" outcomes.
for db in kratos hydra; do
  psqlA -c "CREATE DATABASE ${db};" >/dev/null 2>&1 || true
  psqlB -c "CREATE DATABASE ${db};" >/dev/null 2>&1 || true
  for fn in psqlA psqlB; do
    $fn -d "$db" -c "CREATE EXTENSION IF NOT EXISTS pg_trgm;"   >/dev/null 2>&1 || true
    $fn -d "$db" -c "CREATE EXTENSION IF NOT EXISTS btree_gin;" >/dev/null 2>&1 || true
    $fn -d "$db" -c "CREATE EXTENSION IF NOT EXISTS spock;"     >/dev/null
  done
done

echo "[2node-soak] run kratos+hydra migrations on both nodes"
docker compose run --rm kratos-migrate-a >/dev/null
docker compose run --rm kratos-migrate-b >/dev/null
docker compose run --rm hydra-migrate-a  >/dev/null
docker compose run --rm hydra-migrate-b  >/dev/null

# Kratos creates a row in `networks` on first boot of `serve` and uses
# that row's UUID as the `nid` (multi-tenancy column) for every identity
# it creates. If kratos-a and kratos-b each create their own networks
# row, Spock will replicate both rows fine — but each kratos's API still
# filters by ITS network's UUID, so an identity created at A is invisible
# to B's API even though the row is sitting in B's postgres. Pre-seed a
# single deterministic network row at A BEFORE either kratos starts;
# Spock replicates it to B; both kratos instances see exactly one row
# and pick it. (Verified: this is the kratos-side gap, not a
# Spock gap; without this pre-seed, repl_a_to_b reports 0 even though
# the rows are present in both postgres DBs.)
SHARED_NID="00000000-0000-4000-8000-000000000001"
# Insert the same UUID on both nodes BEFORE Spock subscriptions are
# configured, with ON CONFLICT DO NOTHING. Spock subscriptions come
# next; the duplicate INSERTs are no-ops at both sides. After Spock is
# wired, both nodes already have the row, so kratos-a and kratos-b
# will each pick it (single existing row → both kratos instances use
# the same nid → identity rows propagate end-to-end through the API).
for db in kratos hydra; do
  # F-002 + ADR 1010 D7: pre-seed the shared `networks` row in BOTH
  # databases on BOTH nodes BEFORE the services start. Critical detail:
  # the timestamp MUST be earlier than 2013-10-07 because Hydra's
  # `hydra-migrate` inserts a default networks row with that hardcoded
  # date, and Hydra's networkx.Manager.Determine() adopts whichever row
  # has the earliest created_at. Use 1970-01-01 so the pre-seed always
  # wins. NOW would lose the tie-break.
  psqlA -d "$db" -tAXc "INSERT INTO networks (id, created_at, updated_at) VALUES ('$SHARED_NID', '1970-01-01 00:00:00', '1970-01-01 00:00:00') ON CONFLICT (id) DO NOTHING;" >/dev/null
  psqlB -d "$db" -tAXc "INSERT INTO networks (id, created_at, updated_at) VALUES ('$SHARED_NID', '1970-01-01 00:00:00', '1970-01-01 00:00:00') ON CONFLICT (id) DO NOTHING;" >/dev/null
done
echo "[2node-soak] shared network ${SHARED_NID} pre-seeded on both nodes (kratos+hydra) @ 1970-01-01"

echo "[2node-soak] configure Spock node + subscriptions for kratos + hydra"
# DSN form for spock to reach the OTHER node from inside this node.
DSN_A_TO_B="host=postgres-b port=5432 user=postgres password=${POSTGRES_SUPERUSER_PASSWORD}"
DSN_B_TO_A="host=postgres-a port=5432 user=postgres password=${POSTGRES_SUPERUSER_PASSWORD}"

for db in kratos hydra; do
  # Provider-side DSNs need dbname=<db>.
  PROV_A="${DSN_A_TO_B} dbname=${db}"
  PROV_B="${DSN_B_TO_A} dbname=${db}"
  # Each node creates its own logical node, identifying itself.
  psqlA -d "$db" -c "SELECT spock.node_create(node_name := 'node_a_${db}', dsn := 'host=postgres-a port=5432 dbname=${db} user=postgres password=${POSTGRES_SUPERUSER_PASSWORD}');" >/dev/null
  psqlB -d "$db" -c "SELECT spock.node_create(node_name := 'node_b_${db}', dsn := 'host=postgres-b port=5432 dbname=${db} user=postgres password=${POSTGRES_SUPERUSER_PASSWORD}');" >/dev/null

  # Add tables to the default repset one by one. Skip tables that lack
  # a PRIMARY KEY (e.g. schema_migration) — those don't need cross-node
  # replication anyway since each node migrates independently. The
  # critical tables (the ones whose A→B / B→A propagation the soak
  # actually verifies) MUST succeed; if any of them fail to add to the
  # repset, the whole soak fails fast — silent skip would let the soak
  # run for 2 hours producing repl_*=0 every tick without ever telling
  # us why.
  if [ "$db" = "kratos" ]; then
    CRITICAL_TABLES="identities,identity_credentials,identity_credential_identifiers,networks,selfservice_login_flows,selfservice_registration_flows,selfservice_settings_flows,selfservice_recovery_flows,selfservice_verification_flows,sessions"
  else
    CRITICAL_TABLES="hydra_client,hydra_oauth2_access,hydra_oauth2_refresh,hydra_oauth2_code,hydra_jwk"
  fi
  ADD_TABLES_SQL=$(cat <<PLPGSQL
DO \$\$
DECLARE
  t text;
  skipped int := 0;
  added int := 0;
  failed_critical text := '';
  critical_set text[] := string_to_array('${CRITICAL_TABLES}', ',');
BEGIN
  FOR t IN SELECT tablename FROM pg_tables WHERE schemaname='public' LOOP
    BEGIN
      PERFORM spock.repset_add_table('default', format('%I.%I', 'public', t)::regclass);
      added := added + 1;
    EXCEPTION WHEN OTHERS THEN
      IF t = ANY(critical_set) THEN
        failed_critical := failed_critical || t || '(' || SQLERRM || '); ';
      END IF;
      skipped := skipped + 1;
    END;
  END LOOP;
  IF length(failed_critical) > 0 THEN
    RAISE EXCEPTION 'critical tables failed to add to default repset: %', failed_critical;
  END IF;
  RAISE NOTICE 'repset_add: added=%, skipped=%', added, skipped;
END;
\$\$;
PLPGSQL
)
  # Capture output AND exit code from a single run. A second silent re-run
  # would self-trigger: by the second pass every table is "already a member
  # of replication set", which the EXCEPTION block records as failed for
  # every critical table — turning a successful first pass into a false
  # FATAL. Use ON_ERROR_STOP + PIPESTATUS on the first call instead.
  out_a=$(psqlA -v ON_ERROR_STOP=1 -d "$db" -c "$ADD_TABLES_SQL" 2>&1); rc_a=$?
  echo "$out_a" | grep -E "(NOTICE|repset_add|critical tables)" | tail -5
  if [ $rc_a -ne 0 ]; then
    echo "[2node-soak] FATAL: A's repset_add for $db failed (rc=$rc_a): $out_a"
    exit 1
  fi
  out_b=$(psqlB -v ON_ERROR_STOP=1 -d "$db" -c "$ADD_TABLES_SQL" 2>&1); rc_b=$?
  echo "$out_b" | grep -E "(NOTICE|repset_add|critical tables)" | tail -5
  if [ $rc_b -ne 0 ]; then
    echo "[2node-soak] FATAL: B's repset_add for $db failed (rc=$rc_b): $out_b"
    exit 1
  fi

  # Subscribe each side to the other.
  psqlA -d "$db" -c "SELECT spock.sub_create(subscription_name := 'sub_a_from_b_${db}', provider_dsn := '${PROV_A}', synchronize_data := false);" >/dev/null
  psqlB -d "$db" -c "SELECT spock.sub_create(subscription_name := 'sub_b_from_a_${db}', provider_dsn := '${PROV_B}', synchronize_data := false);" >/dev/null
done

echo "[2node-soak] up kratos-a, kratos-b, hydra-a, hydra-b"
docker compose up -d kratos-a kratos-b hydra-a hydra-b

# Wait for all four /health/ready (5 min ceiling).
READY=0
for _ in $(seq 1 150); do
  if curl -sS --max-time 5 -f -o /dev/null "http://127.0.0.1:${KRATOS_A_ADM}/admin/health/ready" \
     && curl -sS --max-time 5 -f -o /dev/null "http://127.0.0.1:${KRATOS_B_ADM}/admin/health/ready" \
     && curl -sS --max-time 5 -f -o /dev/null "http://127.0.0.1:${HYDRA_A_ADM}/health/ready" \
     && curl -sS --max-time 5 -f -o /dev/null "http://127.0.0.1:${HYDRA_B_ADM}/health/ready"; then
    READY=1; break
  fi
  sleep 2
done
if [ "$READY" != "1" ]; then
  echo "[2node-soak] FATAL: not all four daemons reached ready in 5min"
  docker compose logs --tail=80 kratos-a kratos-b hydra-a hydra-b
  exit 1
fi

# F-013 mitigation — start mgmt-host-compose-watch (production pattern)
# at the soak's compose dir. With the watcher running, kill_kratos_a nemesis
# events recover within ≤30s on the watcher's next poll cycle, matching
# production. Without it, kratos-a stays dead.
#
# Poll interval shortened to 15s (vs production default 30s) so recovery
# lands inside the next 30s tick window — gives clean PASS/FAIL signal in
# the JSONL rather than an off-by-one tick race. Production stays at 30s.
#
# Watcher will also re-run kratos-migrate-* / hydra-migrate-* (which exit
# 0 with restart: "no") on each poll; idempotent. Same shape as production
# mgmt-host watcher behavior. Cost: minimal CPU.
WATCHER_SCRIPT="${WATCHER_SCRIPT:-$REPO_ROOT/ops/ansible/roles/mgmt-host-compose-watch/files/mgmt-host-compose-watch.sh}"
WATCHER_LOG="$WORK/mgmt-host-compose-watch.log"
if [ -f "$WATCHER_SCRIPT" ]; then
  echo "[2node-soak] starting compose-watch at $WORK"
  # Allowlist explicitly excludes the four *-migrate services (restart: "no",
  # exit 0 by design). Without this filter the watcher revives them every
  # poll cycle, destabilizing kratos-a.
  MGMT_HOST_COMPOSE_DIR="$WORK" \
    MGMT_HOST_COMPOSE_WATCH_INTERVAL=15 \
    MGMT_HOST_COMPOSE_WATCH_SERVICES="kratos-a kratos-b hydra-a hydra-b" \
    bash "$WATCHER_SCRIPT" >"$WATCHER_LOG" 2>&1 &
  WATCHER_PID=$!
  echo "[2node-soak] compose-watch pid=$WATCHER_PID log=$WATCHER_LOG (allowlist: kratos-a kratos-b hydra-a hydra-b)"
  # Give watcher a few seconds to discover the compose stack and log
  # "starting; compose dir=..." before the soak begins.
  sleep 3
  if ! kill -0 "$WATCHER_PID" 2>/dev/null; then
    echo "[2node-soak] FATAL: compose-watch exited during startup; tail of log:"
    tail -20 "$WATCHER_LOG" || true
    exit 1
  fi
else
  echo "[2node-soak] WARN: compose-watch script not found at $WATCHER_SCRIPT"
  echo "[2node-soak] WARN: kill_kratos_a nemesis recovery will fail"
fi

# Convert SOAK_DURATION → seconds.
DURATION_SEC=$(python3 -c "
import re,sys
s='$SOAK_DURATION'
m=re.match(r'(\d+)([smh]?)', s)
n=int(m.group(1))
unit=m.group(2) or 's'
print(n * {'s':1,'m':60,'h':3600}[unit])
")
START=$(date +%s)
END=$((START + DURATION_SEC))

# Grace period: /health/ready can flip true while docker port forwarding
# is still settling on a busy host. Wait briefly for the actual identity-
# create endpoint to accept a probe POST before declaring loop-ready.
for _ in $(seq 1 30); do
  PROBE=$(curl -sS --max-time 5 -o /dev/null -w "%{http_code}" "http://127.0.0.1:${KRATOS_A_ADM}/admin/identities" 2>/dev/null || echo 000)
  # Any HTTP response (even 405 for GET-vs-POST or 400) means the listener is up.
  case "$PROBE" in 000|7) sleep 1;; *) break;; esac
done

echo "[2node-soak] running for ${SOAK_DURATION} (${DURATION_SEC}s)"
echo "[2node-soak] evidence appended to $EVIDENCE_FILE"

# Tick loop runs WITHOUT set -e: a transient curl/psql failure on any one
# tick must not kill the entire soak. The whole point of soaking is to
# capture intermittent failures as evidence — exiting on first failure
# defeats that. set -u stays on so undefined-variable bugs still surface.
set +e

ITER=0
NEMESIS_FIRE_COUNT=0
NEMESIS_RECOVERY_PENDING=0
while [ "$(date +%s)" -lt "$END" ]; do
  ITER=$((ITER + 1))
  TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  ELAPSED=$(($(date +%s) - START))

  # ─── Nemesis injection (opt-in via SOAK_NEMESIS=1) ────────────────────
  # Fires every NEMESIS_INTERVAL ticks, alternating between two failure
  # types. Records the event tag in JSONL so the summarizer can split
  # nemesis-tick metrics from steady-state metrics. The first post-event
  # tick is tagged nemesis_in_recovery=1 to surface lag-catchup behavior
  # without conflating it with the event tick itself.
  NEMESIS_EVENT=""
  NEMESIS_IN_RECOVERY=$NEMESIS_RECOVERY_PENDING
  NEMESIS_RECOVERY_PENDING=0
  if [ "$SOAK_NEMESIS" = "1" ] && [ "$ITER" -gt 1 ] && [ $((ITER % NEMESIS_INTERVAL)) -eq 0 ]; then
    NEMESIS_FIRE_COUNT=$((NEMESIS_FIRE_COUNT + 1))
    if [ $((NEMESIS_FIRE_COUNT % 2)) -eq 1 ]; then
      # pause_postgres_b: stalls B-side writes + Spock replay for the
      # configured hold window. ADR 1010 D2 (lifespan budget) + D5
      # (drift-watch) coverage.
      NEMESIS_EVENT="pause_postgres_b_${NEMESIS_HOLD_S}s"
      docker pause "$PG_B" >/dev/null 2>&1 || true
      sleep "$NEMESIS_HOLD_S"
      docker unpause "$PG_B" >/dev/null 2>&1 || true
    else
      # kill_kratos_a: validates ADR 1010 D3 (service-side bounded retry)
      # by forcing a kratos-a crash; compose's restart: unless-stopped
      # brings it back. The tick measures the immediate impact; the
      # next tick (nemesis_in_recovery=1) measures recovery.
      NEMESIS_EVENT="kill_kratos_a"
      KA_CID=$(docker compose ps -q kratos-a 2>/dev/null || echo)
      [ -n "$KA_CID" ] && docker kill "$KA_CID" >/dev/null 2>&1 || true
    fi
    NEMESIS_RECOVERY_PENDING=1
  fi

  # Identity create at A.
  EMAIL_A="soak-a-${ITER}@orysoak.local"
  ID_A=$(curl -sS --max-time 5 -X POST "http://127.0.0.1:${KRATOS_A_ADM}/admin/identities" \
    -H "Content-Type: application/json" \
    -d "{\"schema_id\":\"default\",\"traits\":{\"email\":\"${EMAIL_A}\",\"name\":\"A${ITER}\"},\"credentials\":{\"password\":{\"config\":{\"password\":\"SoakA${ITER}!password\"}}}}" 2>/dev/null \
    | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('id',''))" 2>/dev/null || echo)
  CREATE_A_OK=0; [ -n "$ID_A" ] && CREATE_A_OK=1

  # Wait briefly for Spock replication.
  sleep 1

  # Verify the identity from A at TWO layers:
  # - repl_a_to_b_db: row reached postgres-b (Spock did its job)
  # - repl_a_to_b_api: kratos-b's API can resolve it (kratos's multi-
  #   tenancy `nid` filter agrees too — depends on the shared-network
  #   pre-seed at startup)
  # Splitting these isolates "Spock didn't replicate" from "kratos's
  # multi-tenant model lost the row" in the post-soak decision review.
  REPL_A_TO_B_DB=0
  REPL_A_TO_B_API=0
  if [ -n "$ID_A" ]; then
    DB_HIT=$(psqlB -d kratos -tAXc "SELECT count(*) FROM identities WHERE id = '${ID_A}';" 2>/dev/null | tr -d ' ')
    [ "$DB_HIT" = "1" ] && REPL_A_TO_B_DB=1
    HTTP_B=$(curl -sS --max-time 5 -o /dev/null -w "%{http_code}" "http://127.0.0.1:${KRATOS_B_ADM}/admin/identities/${ID_A}" 2>/dev/null || echo 000)
    [ "$HTTP_B" = "200" ] && REPL_A_TO_B_API=1
  fi

  # Client create at B.
  CLIENT_B=$(curl -sS --max-time 5 -X POST "http://127.0.0.1:${HYDRA_B_ADM}/admin/clients" \
    -H "Content-Type: application/json" \
    -d "{\"client_name\":\"soak-b-${ITER}\",\"grant_types\":[\"client_credentials\"],\"scope\":\"read\",\"token_endpoint_auth_method\":\"client_secret_post\"}" 2>/dev/null || echo "{}")
  CID_B=$(echo "$CLIENT_B" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('client_id',''))" 2>/dev/null)
  CSEC_B=$(echo "$CLIENT_B" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('client_secret',''))" 2>/dev/null)
  CREATE_B_OK=0; [ -n "$CID_B" ] && [ -n "$CSEC_B" ] && CREATE_B_OK=1

  sleep 1

  # Verify Hydra client created at B at TWO layers (same split as
  # Kratos: DB-row replication vs API resolvability). Hydra has its own
  # multi-tenancy story (nid columns on hydra_client and friends) that
  # may have analogous gotchas to Kratos networks.
  REPL_B_TO_A_DB=0
  REPL_B_TO_A_API=0
  if [ -n "$CID_B" ]; then
    DB_HIT_HYDRA=$(psqlA -d hydra -tAXc "SELECT count(*) FROM hydra_client WHERE id = '${CID_B}';" 2>/dev/null | tr -d ' ')
    [ "$DB_HIT_HYDRA" = "1" ] && REPL_B_TO_A_DB=1
    HTTP_A=$(curl -sS --max-time 5 -o /dev/null -w "%{http_code}" "http://127.0.0.1:${HYDRA_A_ADM}/admin/clients/${CID_B}" 2>/dev/null || echo 000)
    [ "$HTTP_A" = "200" ] && REPL_B_TO_A_API=1
  fi

  # Cross-node token usage: token issued at B, introspected at A.
  TOKEN_OK=0; INTROSPECT_CROSS=0
  if [ "$CREATE_B_OK" = "1" ]; then
    TOK=$(curl -sS --max-time 5 -X POST "http://127.0.0.1:${HYDRA_B_PUB}/oauth2/token" \
      -H "Content-Type: application/x-www-form-urlencoded" \
      -d "grant_type=client_credentials&client_id=${CID_B}&client_secret=${CSEC_B}&scope=read" 2>/dev/null \
      | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('access_token',''))" 2>/dev/null)
    [ -n "$TOK" ] && TOKEN_OK=1
    if [ -n "$TOK" ]; then
      ACTIVE=$(curl -sS --max-time 5 -X POST "http://127.0.0.1:${HYDRA_A_ADM}/admin/oauth2/introspect" \
        -H "Content-Type: application/x-www-form-urlencoded" \
        -d "token=${TOK}" 2>/dev/null \
        | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('active',False))" 2>/dev/null)
      [ "$ACTIVE" = "True" ] && INTROSPECT_CROSS=1
    fi
  fi

  # ─── A14: Kratos login flow row replication A→B (sticky-routing risk) ───
  # Production motif: a flow created on node A, completed via the next
  # request landing on node B. If the flow row hasn't replicated yet,
  # node B returns "flow not found" and the user is bounced back to start.
  # Mitigation in production = sticky-per-flow routing at Traefik (cookie
  # or path-based session affinity); the soak just measures the race.
  FLOW_OUT=$(curl -sS --max-time 5 -X GET -H "Accept: application/json" "http://127.0.0.1:${KRATOS_A_PUB}/self-service/login/api" 2>/dev/null || echo "{}")
  FLOW_ID=$(echo "$FLOW_OUT" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('id',''))" 2>/dev/null)
  FLOW_CREATE_A_OK=0; [ -n "$FLOW_ID" ] && FLOW_CREATE_A_OK=1
  FLOW_REPL_A_TO_B_DB=0
  FLOW_REPL_A_TO_B_API=0
  if [ -n "$FLOW_ID" ]; then
    sleep 1
    DB_FLOW=$(psqlB -d kratos -tAXc "SELECT count(*) FROM selfservice_login_flows WHERE id = '${FLOW_ID}';" 2>/dev/null | tr -d ' ')
    [ "$DB_FLOW" = "1" ] && FLOW_REPL_A_TO_B_DB=1
    FLOW_AT_B=$(curl -sS --max-time 5 -o /dev/null -w "%{http_code}" "http://127.0.0.1:${KRATOS_B_PUB}/self-service/login/flows?id=${FLOW_ID}" 2>/dev/null || echo 000)
    [ "$FLOW_AT_B" = "200" ] && FLOW_REPL_A_TO_B_API=1
  fi

  # ─── A15: Hydra access-token issued at A, introspected at B (forward dir) ───
  # The reverse direction is already exercised above (issue at B,
  # introspect at A). Adding the forward direction proves both A→B and
  # B→A token rows replicate + are introspectable. The OAuth2 auth-code
  # flow (with /oauth2/auth + consent + code exchange) is NOT covered
  # here — it requires a logged-in user via consent UI, out of scope for
  # the headless soak. Captured as residual risk in ADR 1009.
  TOKEN_A_OK=0; INTROSPECT_A_TO_B=0
  if [ "$CREATE_B_OK" = "1" ]; then
    TOK_A=$(curl -sS --max-time 5 -X POST "http://127.0.0.1:${HYDRA_A_PUB}/oauth2/token" \
      -H "Content-Type: application/x-www-form-urlencoded" \
      -d "grant_type=client_credentials&client_id=${CID_B}&client_secret=${CSEC_B}&scope=read" 2>/dev/null \
      | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('access_token',''))" 2>/dev/null)
    [ -n "$TOK_A" ] && TOKEN_A_OK=1
    if [ -n "$TOK_A" ]; then
      ACTIVE_A_AT_B=$(curl -sS --max-time 5 -X POST "http://127.0.0.1:${HYDRA_B_ADM}/admin/oauth2/introspect" \
        -H "Content-Type: application/x-www-form-urlencoded" \
        -d "token=${TOK_A}" 2>/dev/null \
        | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('active',False))" 2>/dev/null)
      [ "$ACTIVE_A_AT_B" = "True" ] && INTROSPECT_A_TO_B=1
    fi
  fi

  # ─── A16: concurrent-mutation LWW probe ─────────────────────────────────
  # Production motif: same identity X mutated on A (password change) and
  # on B (admin updates email) within the same Spock heartbeat. Spock 5's
  # default is last-write-wins per row → one update's column-set wins
  # entirely; the other is silently dropped. The probe writes different
  # values to traits.name on A and B nearly simultaneously, waits for
  # convergence, and emits TWO fields:
  # - concurrent_converged: both nodes agree on a single value (Spock did
  #   converge to one of the two writes — i.e. LWW resolved)
  # - concurrent_data_loss: one of the two writes was silently dropped
  #   (which is the same condition as converged=1, since the loser's
  #    value never appears anywhere). Tracked as a separate field for
  #    decision-review readability.
  CONCURRENT_CONVERGED=0
  CONCURRENT_DATA_LOSS=0
  if [ "$REPL_A_TO_B_DB" = "1" ]; then
    # Wait by-pid (NOT naked `wait`): the soak script has a long-running
    # background watcher. `wait` with no args would
    # block forever on the watcher's infinite loop.
    psqlA -d kratos -c "UPDATE identities SET traits = jsonb_set(traits, '{name}', '\"A_mod_${ITER}\"'::jsonb) WHERE id = '${ID_A}';" >/dev/null 2>&1 &
    PSQL_A_PID=$!
    psqlB -d kratos -c "UPDATE identities SET traits = jsonb_set(traits, '{name}', '\"B_mod_${ITER}\"'::jsonb) WHERE id = '${ID_A}';" >/dev/null 2>&1 &
    PSQL_B_PID=$!
    wait "$PSQL_A_PID" "$PSQL_B_PID"
    sleep 2
    NAME_AT_A=$(psqlA -d kratos -tAXc "SELECT traits->>'name' FROM identities WHERE id = '${ID_A}';" 2>/dev/null | tr -d ' \n')
    NAME_AT_B=$(psqlB -d kratos -tAXc "SELECT traits->>'name' FROM identities WHERE id = '${ID_A}';" 2>/dev/null | tr -d ' \n')
    if [ -n "$NAME_AT_A" ] && [ "$NAME_AT_A" = "$NAME_AT_B" ]; then
      CONCURRENT_CONVERGED=1
      CONCURRENT_DATA_LOSS=1
    fi
  fi

  # F3 is a cross-site observation, not proof of distributed uniqueness.
  # Preserve each status and accepted identity ID. A transport/server failure
  # must never count as a uniqueness rejection. Use fresh per-run identities.
  F3_JSON=$(python3 "$REPO_ROOT/ops/ansible/roles/ory/tests/soak/concurrent_identity.py" \
    --admin-a "http://127.0.0.1:${KRATOS_A_ADM}" \
    --admin-b "http://127.0.0.1:${KRATOS_B_ADM}" \
    --email "soak-f3-${RUN_ID}-${ITER}@example.org")

  # Spock replication lag: query each subscription's lag from pg_stat_replication.
  LAG_A_BYTES=$(psqlA -d kratos -tAXc "SELECT COALESCE(pg_wal_lsn_diff(pg_current_wal_lsn(), replay_lsn)::text,'0') FROM pg_stat_replication LIMIT 1;" 2>/dev/null | tr -d ' ' || echo 0)
  LAG_B_BYTES=$(psqlB -d kratos -tAXc "SELECT COALESCE(pg_wal_lsn_diff(pg_current_wal_lsn(), replay_lsn)::text,'0') FROM pg_stat_replication LIMIT 1;" 2>/dev/null | tr -d ' ' || echo 0)
  LAG_A_BYTES=${LAG_A_BYTES:-0}
  LAG_B_BYTES=${LAG_B_BYTES:-0}

  # F-013 watcher liveness — 1 if the in-process compose-watch process
  # is still alive (which is what makes kill_kratos_a recovery work),
  # 0 if it died. If 0 across nemesis events, A9 cannot pass even if
  # kratos_a_health later reads 200 by coincidence.
  if [ -n "${WATCHER_PID:-}" ] && kill -0 "$WATCHER_PID" 2>/dev/null; then
    COMPOSE_WATCHER_ALIVE=1
  else
    COMPOSE_WATCHER_ALIVE=0
  fi

  # Health.
  HK_A=$(curl -sS --max-time 5 -o /dev/null -w "%{http_code}" "http://127.0.0.1:${KRATOS_A_ADM}/admin/health/ready" 2>/dev/null || echo 000)
  HK_B=$(curl -sS --max-time 5 -o /dev/null -w "%{http_code}" "http://127.0.0.1:${KRATOS_B_ADM}/admin/health/ready" 2>/dev/null || echo 000)
  HH_A=$(curl -sS --max-time 5 -o /dev/null -w "%{http_code}" "http://127.0.0.1:${HYDRA_A_ADM}/health/ready" 2>/dev/null || echo 000)
  HH_B=$(curl -sS --max-time 5 -o /dev/null -w "%{http_code}" "http://127.0.0.1:${HYDRA_B_ADM}/health/ready" 2>/dev/null || echo 000)

  python3 -c "
import json
print(json.dumps({
  'ts':'$TS','iter':$ITER,'elapsed_s':$ELAPSED,
  'create_a_ok':$CREATE_A_OK,
  'repl_a_to_b_db':$REPL_A_TO_B_DB,
  'repl_a_to_b_api':$REPL_A_TO_B_API,
  'create_b_ok':$CREATE_B_OK,
  'repl_b_to_a_db':$REPL_B_TO_A_DB,
  'repl_b_to_a_api':$REPL_B_TO_A_API,
  'token_ok':$TOKEN_OK,
  'introspect_cross_node':$INTROSPECT_CROSS,
  'flow_create_a_ok':$FLOW_CREATE_A_OK,
  'flow_repl_a_to_b_db':$FLOW_REPL_A_TO_B_DB,
  'flow_repl_a_to_b_api':$FLOW_REPL_A_TO_B_API,
  'token_a_ok':$TOKEN_A_OK,
  'introspect_a_to_b':$INTROSPECT_A_TO_B,
  'concurrent_converged':$CONCURRENT_CONVERGED,
  'concurrent_data_loss':$CONCURRENT_DATA_LOSS,
  **json.loads('''$F3_JSON'''),
  'lag_a_bytes':int('$LAG_A_BYTES' or 0),
  'lag_b_bytes':int('$LAG_B_BYTES' or 0),
  'kratos_a_health':int('$HK_A'),
  'kratos_b_health':int('$HK_B'),
  'hydra_a_health':int('$HH_A'),
  'hydra_b_health':int('$HH_B'),
  'nemesis_event':'$NEMESIS_EVENT',
  'nemesis_in_recovery':$NEMESIS_IN_RECOVERY,
  'compose_watcher_alive':$COMPOSE_WATCHER_ALIVE,
}))
" >> "$EVIDENCE_FILE"

  sleep $TICK_INTERVAL
done

echo "[2node-soak] complete. wrote ${ITER} ticks to $EVIDENCE_FILE"
echo "[2node-soak] last 5 ticks:"
tail -5 "$EVIDENCE_FILE"
