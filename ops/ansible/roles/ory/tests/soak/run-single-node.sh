#!/usr/bin/env bash
# Single-node Ory soak — validates Kratos + Hydra stability on
# postgres-spock+contribs over a sustained churn window.
#
# Purpose: feed evidence into the post-soak Ory-vs-alternatives decision
# review (docs/project/idp-decision-review.md). The 2-node Spock-replicated
# version (run-2node-spock-soak.sh, scaffold-only for now) validates the
# additional cross-Router-A/B replication behavior in Phase 2.
#
# Default duration: 2 hours. Override with $SOAK_DURATION (e.g. 24h).
#
# Run:
#   ops/ansible/roles/ory/tests/soak/run-single-node.sh
#
# Output: appends one JSON line per tick to
#   docs/reports/2026-04-25-ory-soak-evidence.jsonl
# That file feeds the decision-review evidence rollup.
#
# What's exercised:
#   - Identity create churn (every 30s)
#   - OAuth2 client create + token grant churn (every 30s)
#   - Token introspect (every 30s on the most-recent token)
#   - Health checks (every 60s)
# What's NOT exercised (Phase 2):
#   - Spock multi-master replication of kratos/hydra rows
#   - Cross-node conflict resolution
#   - Restart/failover behavior

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../../../../../.." && pwd)"
SOAK_DURATION="${SOAK_DURATION:-2h}"
TODAY="$(date -u +%Y-%m-%d)"
# Each run writes to a TODAY-stamped file. Earlier hardcoded
# "" caused every CI run to APPEND to the historical
# reference evidence file, and the smoke workflow's `ls | head -1`
# would then pick the oldest committed file (alphabetically first)
# and summarize that — making the smoke gate read stale data instead
# of the freshly-produced soak. Mirrors run-2node-spock.sh's pattern.
EVIDENCE_FILE="${EVIDENCE_FILE:-$REPO_ROOT/docs/reports/${TODAY}-ory-soak-evidence.jsonl}"
mkdir -p "$(dirname "$EVIDENCE_FILE")" || { echo "FATAL: cannot create evidence dir for $EVIDENCE_FILE"; exit 1; }
TICK_INTERVAL=30  # seconds

WORK="$(mktemp -d -t ory-soak-XXXXXX)"
trap 'docker compose -f "$WORK/docker-compose.yml" -f "$WORK/docker-compose.ory.yml" -f "$WORK/docker-compose.dev-override.yml" down -v >/dev/null 2>&1 || true; rm -rf "$WORK"' EXIT

# Reuse fixture's bring-up logic: copy the same files we know work.
cp "$REPO_ROOT/ops/compose/mgmt-host/docker-compose.ory.yml" "$WORK/"

cat > "$WORK/docker-compose.yml" <<'EOF'
services:
  postgres-a:
    image: postgres-spock-test:contribs
    command: ["postgres", "-c", "config_file=/etc/postgresql/postgresql.conf"]
    environment:
      POSTGRES_PASSWORD: ${POSTGRES_SUPERUSER_PASSWORD}
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 2s
      retries: 30
    networks:
      - mgmt
networks:
  mgmt:
    name: mgmt-orysoak-fixture
EOF

cat > "$WORK/docker-compose.dev-override.yml" <<'EOF'
services:
  kratos:
    command: ["serve", "-c", "/etc/config/kratos.yaml", "--dev", "--watch-courier"]
  hydra:
    command: ["serve", "-c", "/etc/config/hydra.yaml", "all", "--dev"]
EOF

mkdir "$WORK/ory"
cat > "$WORK/ory/identity-schema.json" <<'EOF'
{"$id":"local","$schema":"http://json-schema.org/draft-07/schema#","title":"X","type":"object","properties":{"traits":{"type":"object","properties":{"email":{"type":"string","format":"email","ory.sh/kratos":{"credentials":{"password":{"identifier":true}}}},"name":{"type":"string"}},"required":["email"],"additionalProperties":false}}}
EOF

export POSTGRES_SUPERUSER_PASSWORD="orysoak-pg-fixed-aaaa-bbbb-cccc"
export ORY_KRATOS_IMAGE="oryd/kratos:v26.2.0@sha256:2a13bb8d362c7a7ae33bd7c0f5168aee46921f15c916a06346db91c06dc76643"
export ORY_HYDRA_IMAGE="oryd/hydra:v26.2.0@sha256:ff67c7fb5f95074fa53374d41151713554960504b340cd3f95b09e65deaea2a9"
export HYDRA_SECRETS_SYSTEM="orysoak-hydra-system-secret-fixed-32"
export HYDRA_SECRETS_COOKIE="orysoak-hydra-cookie-secret-fixed-32"
# Kratos secrets — docker-compose.ory.yml line 83 hard-requires
# ${KRATOS_SECRETS_COOKIE:?...} and ${KRATOS_SECRETS_CIPHER:?...}.
# Without these the smoke run fails before kratos-migrate starts
# (this is the ory-soak-smoke chronic-red bug).
# CIPHER must be EXACTLY 32 bytes (Kratos enforces; AES-256 key).
# COOKIE has minLength 16; using 32 for parity.
export KRATOS_SECRETS_COOKIE="orysoak-kratos-cookie-secret-fixed32"
export KRATOS_SECRETS_CIPHER="orysoakkratoscipher32bytesxxxxxx"
# selfservice-ui secrets — docker-compose.ory.yml hard-requires
# ${UI_COOKIE_SECRET:?...} + ${UI_CSRF_SECRET:?...} + ${SITE_FQDN_FOR_AUTH:?...}
# since the selfservice-ui + oauth2-provider services are codified into the
# canonical compose.
export UI_COOKIE_SECRET="orysoak-ui-cookie-secret-fixed-32b"
export UI_CSRF_SECRET="orysoak-ui-csrf-secret-fixed-32byt"
export SITE_FQDN_FOR_AUTH="https://auth.soak.local"
export KRATOS_PUBLIC_HOST_PORT=24433
export KRATOS_ADMIN_HOST_PORT=24434
export HYDRA_PUBLIC_HOST_PORT=24444
export HYDRA_ADMIN_HOST_PORT=24445

cat > "$WORK/ory/kratos.yaml" <<EOF
version: v26.2.0
serve:
  public:
    base_url: "http://127.0.0.1:${KRATOS_PUBLIC_HOST_PORT}/"
    cors:
      enabled: true
  admin:
    base_url: http://kratos:4434/
identity:
  default_schema_id: default
  schemas:
    - id: default
      url: file:///etc/config/identity-schema.json
selfservice:
  default_browser_return_url: "http://127.0.0.1:${KRATOS_PUBLIC_HOST_PORT}/"
  flows:
    settings:
      ui_url: "http://127.0.0.1:${KRATOS_PUBLIC_HOST_PORT}/settings"
    login:
      ui_url: "http://127.0.0.1:${KRATOS_PUBLIC_HOST_PORT}/login"
    registration:
      ui_url: "http://127.0.0.1:${KRATOS_PUBLIC_HOST_PORT}/register"
secrets:
  cookie:
    - "orysoak-cookie-secret-fixed-do-not-use"
  cipher:
    # Kratos requires cipher secret length == 32 exactly.
    - "orysoak-cipher-secret-32chr-aaaa"
hashers:
  algorithm: bcrypt
  bcrypt:
    cost: 8
courier:
  smtp:
    connection_uri: "smtps://localhost:1025/?disable_starttls=true"
session:
  lifespan: 24h
EOF

cat > "$WORK/ory/hydra.yaml" <<EOF
serve:
  public:
    port: 4444
  admin:
    port: 4445
urls:
  self:
    issuer: "http://127.0.0.1:${HYDRA_PUBLIC_HOST_PORT}/"
  consent: "http://127.0.0.1:3000/consent"
  login: "http://127.0.0.1:3000/login"
secrets:
  system:
    - "${HYDRA_SECRETS_SYSTEM}"
oidc:
  subject_identifiers:
    supported_types:
      - public
oauth2:
  expose_internal_errors: true
EOF

cd "$WORK"

echo "[soak] bringing up postgres-spock+contribs + kratos + hydra"
docker compose -f docker-compose.yml -f docker-compose.ory.yml pull --ignore-pull-failures >/dev/null 2>&1 || true
docker compose -f docker-compose.yml -f docker-compose.ory.yml up -d postgres-a
for _ in $(seq 1 60); do
  [ "$(docker inspect -f '{{.State.Health.Status}}' "$(docker compose -f docker-compose.yml -f docker-compose.ory.yml ps -q postgres-a)" 2>/dev/null)" = "healthy" ] && break
  sleep 1
done
docker exec -e PGPASSWORD="$POSTGRES_SUPERUSER_PASSWORD" \
  "$(docker compose -f docker-compose.yml -f docker-compose.ory.yml ps -q postgres-a)" \
  psql -U postgres -c "CREATE DATABASE kratos;" >/dev/null 2>&1 || true
docker exec -e PGPASSWORD="$POSTGRES_SUPERUSER_PASSWORD" \
  "$(docker compose -f docker-compose.yml -f docker-compose.ory.yml ps -q postgres-a)" \
  psql -U postgres -c "CREATE DATABASE hydra;" >/dev/null 2>&1 || true
docker compose -f docker-compose.yml -f docker-compose.ory.yml run --rm kratos-migrate >/dev/null
docker compose -f docker-compose.yml -f docker-compose.ory.yml run --rm hydra-migrate >/dev/null
docker compose -f docker-compose.yml -f docker-compose.ory.yml -f docker-compose.dev-override.yml up -d kratos hydra

# Wait for ready (5 min ceiling — first run pulls images).
READY=0
for _ in $(seq 1 150); do
  if curl -sS -f -o /dev/null "http://127.0.0.1:${KRATOS_ADMIN_HOST_PORT}/admin/health/ready" \
     && curl -sS -f -o /dev/null "http://127.0.0.1:${HYDRA_ADMIN_HOST_PORT}/health/ready"; then
    READY=1; break
  fi
  sleep 2
done
if [ "$READY" != "1" ]; then
  echo "[soak] FATAL: kratos+hydra did not become ready within 5min"
  docker compose -f docker-compose.yml -f docker-compose.ory.yml -f docker-compose.dev-override.yml logs --tail=80 kratos hydra
  exit 1
fi

# Convert SOAK_DURATION to seconds.
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

echo "[soak] running for ${SOAK_DURATION} (${DURATION_SEC}s)"
echo "[soak] evidence appended to $EVIDENCE_FILE"

LAST_TOKEN=""
ITER=0

while [ "$(date +%s)" -lt "$END" ]; do
  ITER=$((ITER + 1))
  TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  ELAPSED=$(($(date +%s) - START))

  # Identity create
  ID_HTTP=$(curl -sS -o /dev/null -w "%{http_code}" -X POST "http://127.0.0.1:${KRATOS_ADMIN_HOST_PORT}/admin/identities" \
    -H "Content-Type: application/json" \
    -d "{\"schema_id\":\"default\",\"traits\":{\"email\":\"soak-${ITER}@orysoak.local\",\"name\":\"Soak ${ITER}\"},\"credentials\":{\"password\":{\"config\":{\"password\":\"SoakTest${ITER}!password\"}}}}" 2>/dev/null || echo 000)

  # OAuth2 client create
  CLIENT_OUT=$(curl -sS -X POST "http://127.0.0.1:${HYDRA_ADMIN_HOST_PORT}/admin/clients" \
    -H "Content-Type: application/json" \
    -d "{\"client_name\":\"soak-${ITER}\",\"grant_types\":[\"client_credentials\"],\"scope\":\"read\",\"token_endpoint_auth_method\":\"client_secret_post\"}" 2>/dev/null || echo "{}")
  CID=$(echo "$CLIENT_OUT" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('client_id',''))" 2>/dev/null)
  CSEC=$(echo "$CLIENT_OUT" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('client_secret',''))" 2>/dev/null)
  CLIENT_OK=0; [ -n "$CID" ] && [ -n "$CSEC" ] && CLIENT_OK=1

  # Token grant
  TOKEN_OK=0
  if [ "$CLIENT_OK" = "1" ]; then
    TOK=$(curl -sS -X POST "http://127.0.0.1:${HYDRA_PUBLIC_HOST_PORT}/oauth2/token" \
      -H "Content-Type: application/x-www-form-urlencoded" \
      -d "grant_type=client_credentials&client_id=$CID&client_secret=$CSEC&scope=read" 2>/dev/null \
      | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('access_token',''))" 2>/dev/null)
    [ -n "$TOK" ] && TOKEN_OK=1 && LAST_TOKEN="$TOK"
  fi

  # Introspect last token
  INTROSPECT_OK=0
  if [ -n "$LAST_TOKEN" ]; then
    ACTIVE=$(curl -sS -X POST "http://127.0.0.1:${HYDRA_ADMIN_HOST_PORT}/admin/oauth2/introspect" \
      -H "Content-Type: application/x-www-form-urlencoded" \
      -d "token=$LAST_TOKEN" 2>/dev/null \
      | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('active',False))" 2>/dev/null)
    [ "$ACTIVE" = "True" ] && INTROSPECT_OK=1
  fi

  # Health
  KRATOS_HEALTH=$(curl -sS -o /dev/null -w "%{http_code}" "http://127.0.0.1:${KRATOS_ADMIN_HOST_PORT}/admin/health/ready" 2>/dev/null || echo 000)
  HYDRA_HEALTH=$(curl -sS -o /dev/null -w "%{http_code}" "http://127.0.0.1:${HYDRA_ADMIN_HOST_PORT}/health/ready" 2>/dev/null || echo 000)

  # JSON evidence line
  python3 -c "
import json
print(json.dumps({
  'ts':'$TS','iter':$ITER,'elapsed_s':$ELAPSED,
  'identity_create_http':int('$ID_HTTP'),
  'client_create_ok':$CLIENT_OK,
  'token_grant_ok':$TOKEN_OK,
  'introspect_ok':$INTROSPECT_OK,
  'kratos_health':int('$KRATOS_HEALTH'),
  'hydra_health':int('$HYDRA_HEALTH')
}))
" >> "$EVIDENCE_FILE"

  sleep $TICK_INTERVAL
done

echo "[soak] complete. wrote $((ITER)) ticks to $EVIDENCE_FILE"
echo "[soak] last 5 ticks:"
tail -5 "$EVIDENCE_FILE"
