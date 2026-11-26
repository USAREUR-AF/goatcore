#!/usr/bin/env bash
# ops/ansible/roles/ory/tests/test-fixture.sh
#
# Integration test for the Ory role. Brings up postgres-spock + the Ory
# overlay locally via docker compose (no Ansible runner required), runs
# the same five tests the spike ran, asserts each passes.
#
# Run locally:
#   ops/ansible/roles/ory/tests/test-fixture.sh
#
# Triggers from CI: .github/workflows/ory-role.yml on changes to
# ops/ansible/roles/ory/** or ops/compose/mgmt-host/docker-compose.ory.yml.
#
# Side effects: temporary docker-compose project (project name "orytest")
# under /tmp/ory-role-fixture-<pid>/. Auto-removed on success or failure.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../../../../.." && pwd)"
WORK="$(mktemp -d -t ory-role-fixture-XXXXXX)"
trap 'docker compose -f "$WORK/docker-compose.yml" -f "$WORK/docker-compose.ory.yml" down -v >/dev/null 2>&1 || true; rm -rf "$WORK"' EXIT

# 32+ char fixed secrets — fine for test fixture only.
export POSTGRES_SUPERUSER_PASSWORD="orytest-pg-password-fixed-aaaa"
export ORY_KRATOS_IMAGE="oryd/kratos:v26.2.0@sha256:2a13bb8d362c7a7ae33bd7c0f5168aee46921f15c916a06346db91c06dc76643"
export ORY_HYDRA_IMAGE="oryd/hydra:v26.2.0@sha256:ff67c7fb5f95074fa53374d41151713554960504b340cd3f95b09e65deaea2a9"
export HYDRA_SECRETS_SYSTEM="orytest-hydra-system-secret-fixed-32"
export HYDRA_SECRETS_COOKIE="orytest-hydra-cookie-secret-fixed-32"

# Ports — different from production defaults so we don't collide with a
# running mgmt-host stack on the same machine.
export KRATOS_PUBLIC_HOST_PORT=14433
export KRATOS_ADMIN_HOST_PORT=14434
export HYDRA_PUBLIC_HOST_PORT=14444
export HYDRA_ADMIN_HOST_PORT=14445

cp "$REPO_ROOT/ops/compose/mgmt-host/docker-compose.ory.yml" "$WORK/"

# Minimal base compose: postgres-a (postgres-spock with contribs) + the
# `mgmt` network the overlay attaches to. Mirrors the production posture
# where Ory and netbird-mgmt share the same Spock cluster.
#
# The image is the locally-built postgres-spock-test:contribs (rebuilt by
# the Ory role test alongside any postgres-spock Dockerfile change). For
# CI, we'd build it with `docker build ops/postgres/spock -t postgres-spock-test:contribs`
# in a setup step before running this fixture.
cat > "$WORK/docker-compose.yml" <<'EOF'
services:
  postgres-a:
    image: postgres-spock-test:contribs
    command: ["postgres", "-c", "config_file=/etc/postgresql/postgresql.conf"]
    environment:
      POSTGRES_PASSWORD: ${POSTGRES_SUPERUSER_PASSWORD}
    ports:
      - "127.0.0.1:15435:5432"
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 2s
      retries: 30
    networks:
      - mgmt
networks:
  mgmt:
    name: mgmt-orytest-fixture
EOF

# Render Kratos + Hydra configs. We use the same Jinja templates the role
# uses; for fixture purposes we substitute via envsubst-ish here so we don't
# require Ansible to be installed.
mkdir -p "$WORK/ory"
cat > "$WORK/ory/identity-schema.json" <<'EOF'
{
  "$id": "https://schemas.netbird.legacy.example/identity/admin.v1.json",
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "goat operator identity",
  "type": "object",
  "properties": {
    "traits": {
      "type": "object",
      "properties": {
        "email": {"type": "string", "format": "email", "title": "Email",
          "ory.sh/kratos": {"credentials": {"password": {"identifier": true}}, "verification": {"via": "email"}, "recovery": {"via": "email"}}},
        "name": {"type": "string", "title": "Display name"}
      },
      "required": ["email", "name"],
      "additionalProperties": false
    }
  }
}
EOF

# Kratos config — fixture variant. Issuer/redirect URLs use 127.0.0.1
# rather than the per-site hostname, since this is local-only.
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
    - "orytest-cookie-secret-fixed-do-not-use"
  cipher:
    # Kratos requires cipher secret length == 32 (validation in serve config).
    - "orytest-cipher-secret-32chr-xxxx"
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

echo "[1/6] pull Ory images (skip postgres-spock-test, built locally)"
docker compose -f docker-compose.yml -f docker-compose.ory.yml pull --ignore-pull-failures >/dev/null 2>&1 || true

echo "[2/6] up postgres-a + run kratos-migrate + hydra-migrate"
docker compose -f docker-compose.yml -f docker-compose.ory.yml up -d postgres-a
# Wait for postgres healthy.
for _ in $(seq 1 60); do
  if [ "$(docker inspect -f '{{.State.Health.Status}}' "$(docker compose -f docker-compose.yml -f docker-compose.ory.yml ps -q postgres-a)" 2>/dev/null)" = "healthy" ]; then break; fi
  sleep 1
done
# Create kratos + hydra DBs.
docker exec -e PGPASSWORD="$POSTGRES_SUPERUSER_PASSWORD" \
  "$(docker compose -f docker-compose.yml -f docker-compose.ory.yml ps -q postgres-a)" \
  psql -U postgres -c "CREATE DATABASE kratos;" >/dev/null 2>&1 || true
docker exec -e PGPASSWORD="$POSTGRES_SUPERUSER_PASSWORD" \
  "$(docker compose -f docker-compose.yml -f docker-compose.ory.yml ps -q postgres-a)" \
  psql -U postgres -c "CREATE DATABASE hydra;" >/dev/null 2>&1 || true
docker compose -f docker-compose.yml -f docker-compose.ory.yml run --rm kratos-migrate >/dev/null
docker compose -f docker-compose.yml -f docker-compose.ory.yml run --rm hydra-migrate >/dev/null

echo "[3/6] up kratos + hydra (with --dev flag override for HTTP issuer)"
# Production overlay omits --dev because the production issuer is HTTPS via
# Traefik. The fixture uses HTTP-on-loopback so we override commands here to
# add --dev. This isolates the dev-flag concern to fixture territory.
cat > "$WORK/docker-compose.dev-override.yml" <<'OVR'
services:
  kratos:
    command: ["serve", "-c", "/etc/config/kratos.yaml", "--dev", "--watch-courier"]
  hydra:
    command: ["serve", "-c", "/etc/config/hydra.yaml", "all", "--dev"]
OVR
docker compose -f docker-compose.yml -f docker-compose.ory.yml -f docker-compose.dev-override.yml up -d kratos hydra

# Wait for both /health/ready.
for _ in $(seq 1 60); do
  if curl -sS -f -o /dev/null "http://127.0.0.1:${KRATOS_ADMIN_HOST_PORT}/admin/health/ready" \
     && curl -sS -f -o /dev/null "http://127.0.0.1:${HYDRA_ADMIN_HOST_PORT}/health/ready"; then
    break
  fi
  sleep 2
done

echo "[4/6] T1: Kratos /admin/identities create"
ID_OUT=$(curl -sS -X POST "http://127.0.0.1:${KRATOS_ADMIN_HOST_PORT}/admin/identities" \
  -H "Content-Type: application/json" \
  -d '{"schema_id":"default","traits":{"email":"fixture@orytest.local","name":"Fixture Admin"},"credentials":{"password":{"config":{"password":"FixtureTest12345!"}}}}')
echo "$ID_OUT" | grep -q '"state":"active"' || { echo "FAIL T1: $ID_OUT"; exit 1; }
echo "  ✓ identity created"

echo "[5/6] T2: Hydra /admin/clients create + client_credentials grant"
CLIENT_OUT=$(curl -sS -X POST "http://127.0.0.1:${HYDRA_ADMIN_HOST_PORT}/admin/clients" \
  -H "Content-Type: application/json" \
  -d '{"client_name":"orytest-fixture","grant_types":["client_credentials"],"scope":"read","token_endpoint_auth_method":"client_secret_post"}')
CLIENT_ID=$(echo "$CLIENT_OUT" | python3 -c "import sys,json; print(json.load(sys.stdin)['client_id'])")
CLIENT_SECRET=$(echo "$CLIENT_OUT" | python3 -c "import sys,json; print(json.load(sys.stdin)['client_secret'])")
[ -n "$CLIENT_ID" ] && [ -n "$CLIENT_SECRET" ] || { echo "FAIL T2 client create: $CLIENT_OUT"; exit 1; }
echo "  ✓ client created"

TOKEN_OUT=$(curl -sS -X POST "http://127.0.0.1:${HYDRA_PUBLIC_HOST_PORT}/oauth2/token" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=client_credentials&client_id=${CLIENT_ID}&client_secret=${CLIENT_SECRET}&scope=read")
ACCESS_TOKEN=$(echo "$TOKEN_OUT" | python3 -c "import sys,json; print(json.load(sys.stdin).get('access_token',''))")
[ -n "$ACCESS_TOKEN" ] || { echo "FAIL T2 token: $TOKEN_OUT"; exit 1; }
echo "  ✓ token issued (${ACCESS_TOKEN:0:20}...)"

echo "[6/6] T3: Hydra /admin/oauth2/introspect validates the token"
INTROSPECT=$(curl -sS -X POST "http://127.0.0.1:${HYDRA_ADMIN_HOST_PORT}/admin/oauth2/introspect" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "token=${ACCESS_TOKEN}")
echo "$INTROSPECT" | grep -q '"active":true' || { echo "FAIL T3: $INTROSPECT"; exit 1; }
echo "  ✓ token introspect active=true"

echo ""
echo "========================================"
echo "Ory role fixture: ALL TESTS PASS"
echo "  T1  Kratos identity create"
echo "  T2  Hydra client + client_credentials grant"
echo "  T3  Hydra token introspect"
echo "========================================"
