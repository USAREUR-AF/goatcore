#!/usr/bin/env bash
# stack.sh — the smallest goatnet the goat CLI can operate end to end, on one
# machine (docs/design/deployment-e2e.md §Suites, the goat suite).
#
#   ops/e2e/suites/goat/stack.sh up   <state-dir>   # build, start, write a goat profile
#   ops/e2e/suites/goat/stack.sh down <state-dir>   # stop everything it started
#
# What runs, and what stands in:
#   portal     the real goat-zenoh-portal (ops/goat-zenoh-portal), registry in Postgres,
#              the router config template the ACL tests use; ACL reload and cert
#              signing use the portal's own fakes (a real reload needs the router
#              container's docker socket; a real signer needs the offline CA).
#   console    the real goat-id-console (ops/goat-id-console), its stores in Postgres
#              (ADR 1165 step A): postgres:16.4 in Docker on 127.0.0.1:18083, or the
#              server named by GOAT_STACK_PG (a libpq URL base, no database name).
#              The onboarding engine `goat admin invite/queue` talks to it.
#   identity   ops/goat-id-console/web/e2e/identity-fixture.mjs — the stand-in for
#              Kratos, Hydra and NetBird management the console's own browser tests
#              use. Minting a person (Kratos identity + NetBird key) is therefore not
#              exercised here; the live tier covers the intake path on a dev goatnet.
#   router     eclipse/zenoh 1.9.0 (digest-pinned, the version goat links) with an
#              mTLS listener and the REST plugin, when Docker is available; the
#              portal then uses the real router's REST plugin instead of its fake.
#              Without Docker the router-backed cases are recorded as xfail with
#              the reason, never as a pass.
#
# Ports are fixed and loopback-only: 18080 portal, 18081 console, 18082 identity,
# 18083 Postgres, 17447 router (mTLS), 18000 router REST.
set -euo pipefail

MODE="${1:?up|down}"
ST="${2:?state directory}"
REPO="$(cd "$(dirname "$0")/../../../.." && pwd)"
ZENOH_IMAGE="${E2E_ZENOH_IMAGE:-eclipse/zenoh@sha256:157965d71e0bfd0a044d76a985ff0e5c306ad3968929168fb9678cd2a7fec23f}"
TOKEN="${E2E_GOAT_TOKEN:-e2e-operator-token}"
say() { printf '[goat-stack] %s\n' "$*"; }

down() {
  [ -f "$ST/pids" ] && while read -r p; do kill "$p" 2>/dev/null || true; done < "$ST/pids"
  rm -f "$ST/pids"
  if command -v docker >/dev/null 2>&1; then docker rm -f goat-e2e-zenoh goat-e2e-pg >/dev/null 2>&1 || true; fi
}

# The console's two stores: an empty database each, on GOAT_STACK_PG or a
# throwaway postgres:16.4 (the digest the console's CI uses).
pg_up() {
  if [ -z "${GOAT_STACK_PG:-}" ]; then
    docker run -d --name goat-e2e-pg -e POSTGRES_PASSWORD=e2e -p 127.0.0.1:18083:5432 \
      postgres:16.4@sha256:e62fbf9d3e2b49816a32c400ed2dba83e3b361e6833e624024309c35d334b412 >/dev/null
    GOAT_STACK_PG="postgres://postgres:e2e@127.0.0.1:18083"
    local i=0
    until docker exec goat-e2e-pg pg_isready -U postgres -h 127.0.0.1 >/dev/null 2>&1; do
      i=$((i+1)); [ "$i" -lt 60 ] || { say "postgres did not start"; docker logs goat-e2e-pg 2>&1 | tail -20; return 1; }
      sleep 1
    done
  fi
  local db
  for db in e2e_onboard e2e_orgs e2e_portal; do
    psql "$GOAT_STACK_PG/postgres" -qc "DROP DATABASE IF EXISTS $db" -c "CREATE DATABASE $db" >/dev/null 2>&1 \
      || docker exec goat-e2e-pg psql -U postgres -qc "DROP DATABASE IF EXISTS $db" -c "CREATE DATABASE $db" >/dev/null
  done
  say "postgres up ($GOAT_STACK_PG)"
}

wait_http() { # url, seconds
  local i=0
  while [ "$i" -lt "$2" ]; do
    curl -fsS -o /dev/null --max-time 2 "$1" 2>/dev/null && return 0
    i=$((i + 1)); sleep 1
  done
  return 1
}

certs() { # a throwaway CA, the router's server cert, goat's client cert
  local d="$ST/pki"; mkdir -p "$d"
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 2 \
    -subj "/CN=goat-e2e-root" -keyout "$d/ca.key" -out "$d/ca.pem" >/dev/null 2>&1
  printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth,clientAuth\n' > "$d/router.ext"
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj "/CN=router.e2e" \
    -keyout "$d/router.key" -out "$d/router.csr" >/dev/null 2>&1
  openssl x509 -req -in "$d/router.csr" -CA "$d/ca.pem" -CAkey "$d/ca.key" -CAcreateserial \
    -days 2 -extfile "$d/router.ext" -out "$d/router.pem" >/dev/null 2>&1
  printf 'extendedKeyUsage=clientAuth\n' > "$d/client.ext"
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj "/CN=e2e-operator.e2e" \
    -keyout "$d/client.key" -out "$d/client.csr" >/dev/null 2>&1
  openssl x509 -req -in "$d/client.csr" -CA "$d/ca.pem" -CAkey "$d/ca.key" -CAcreateserial \
    -days 2 -extfile "$d/client.ext" -out "$d/client.pem" >/dev/null 2>&1
}

router_up() { # 0 = a real router is listening; 1 = none (no Docker)
  command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1 || { say "no Docker: router-backed cases will be xfail"; return 1; }
  cat > "$ST/pki/zenohd.json5" <<'JSON'
{
  mode: "router",
  listen: { endpoints: ["tls/0.0.0.0:7447"] },
  transport: { link: { tls: {
    root_ca_certificate: "/pki/ca.pem",
    listen_private_key: "/pki/router.key",
    listen_certificate: "/pki/router.pem",
    enable_mtls: true,
  } } },
  plugins: { rest: { http_port: 8000 } },
}
JSON
  docker rm -f goat-e2e-zenoh >/dev/null 2>&1 || true
  docker run -d --name goat-e2e-zenoh -p 127.0.0.1:17447:7447 -p 127.0.0.1:18000:8000 \
    -v "$ST/pki:/pki:ro" "$ZENOH_IMAGE" --config /pki/zenohd.json5 >/dev/null
  wait_http "http://127.0.0.1:18000/@/local/router" 60 || { docker logs goat-e2e-zenoh 2>&1 | tail -20; return 1; }
  say "router up (mTLS 127.0.0.1:17447, REST 127.0.0.1:18000)"
}

up() {
  down; mkdir -p "$ST/bin" "$ST/p/ca" "$ST/p/bundles" "$ST/p/recipient-keys" "$ST/p/schemas" "$ST/p/rocksdb"
  rm -f "$ST"/*.db "$ST/router"
  say "building portal and console"
  (cd "$REPO/ops/goat-zenoh-portal" && go build -o "$ST/bin/portal" ./cmd/portal)
  (cd "$REPO/ops/goat-id-console" && go build -o "$ST/bin/console" ./cmd/console)
  certs
  pg_up
  local zenoh_rest="--zenoh-rest-fake"
  if router_up; then zenoh_rest="--zenoh-rest=http://127.0.0.1:18000"; echo real > "$ST/router"; fi
  cp "$REPO/ops/ansible/roles/goat-zenoh/tests/acl-assemble/fixture-live.json5" "$ST/p/acl.json5"
  IDENTITY_PORT=18082 node "$REPO/ops/goat-id-console/web/e2e/identity-fixture.mjs" > "$ST/identity.log" 2>&1 &
  echo $! >> "$ST/pids"
  "$ST/bin/portal" serve --listen 127.0.0.1:18080 --db-dsn "$GOAT_STACK_PG/e2e_portal?sslmode=disable" $zenoh_rest \
    --acl-reload-fake --cert-sign-fake --operator-api-key "$TOKEN" \
    --kratos-admin-url http://127.0.0.1:18082 --acl-config "$ST/p/acl.json5" \
    --acl-storage-dir-prefix "$ST/p/rocksdb" --ca-bundle-dir "$ST/p/ca" \
    --vendor-bundle-dir "$ST/p/bundles" --recipient-keys-dir "$ST/p/recipient-keys" \
    --schema-dir "$ST/p/schemas" > "$ST/portal.log" 2>&1 &
  echo $! >> "$ST/pids"
  env IDP_TENANT_KRATOS_ADMIN_URL=http://127.0.0.1:18082 IDP_TENANT_HYDRA_ADMIN_URL=http://127.0.0.1:18082 \
    LISTEN_ADDR=127.0.0.1:18081 NOTIFY_GOATNET=zagros \
    PERMISSIONS_YAML_PATH="$REPO/ops/permissions/group-permissions.yml" \
    COALITION_FRAMEWORKS_PATH="$REPO/ops/identity/coalition-frameworks.yaml" \
    DEV_BYPASS_AUTH=1 STEP_UP_ENFORCED=false AUDIT_BACKEND=stderr \
    ONBOARD_OPERATOR_API_KEYS="e2e-operator:$TOKEN" ONBOARD_SERVICE_OPERATOR_HANDLES=e2e-operator MEMBERSHIP_AUTHORITY=ungoverned \
    NOTIFY_MGMT_URL=http://127.0.0.1:18082/zagros ENROLL_WATCH_NB_TOKEN=e2e \
    ONBOARD_DB_DSN="$GOAT_STACK_PG/e2e_onboard?sslmode=disable" ORG_DB_DSN="$GOAT_STACK_PG/e2e_orgs?sslmode=disable" \
    PORTAL_INTERNAL_URL=http://127.0.0.1:18080 PORTAL_LOOKUP_TOKEN="$TOKEN" \
    "$ST/bin/console" > "$ST/console.log" 2>&1 &
  echo $! >> "$ST/pids"
  wait_http http://127.0.0.1:18082/healthz 30 || { say "identity fixture did not start"; tail "$ST/identity.log"; exit 1; }
  wait_http http://127.0.0.1:18080/api/health 60 || { say "portal did not start"; tail -20 "$ST/portal.log"; exit 1; }
  wait_http http://127.0.0.1:18081/healthz 60 || { say "console did not start"; tail -20 "$ST/console.log"; exit 1; }
  # The goat profile an operator would have after `goat profile init`: endpoints,
  # the bearer token, and the mTLS material for the router.
  local G="$ST/goathome/contexts/e2e"; rm -rf "$ST/goathome"; mkdir -p "$G"
  {
    echo 'portal_endpoint = "http://127.0.0.1:18080"'
    echo 'onboard_endpoint = "http://127.0.0.1:18081"'
    echo 'router_endpoint = "tls/127.0.0.1:17447"'
    echo 'goatnet = "zagros"'
    echo 'operator_handle = "e2e-operator"'
  } > "$G/profile.toml"
  printf '%s' "$TOKEN" > "$G/portal.token"; chmod 600 "$G/portal.token"
  cp "$ST/pki/client.pem" "$G/mtls.cert.pem"; cp "$ST/pki/client.key" "$G/mtls.key.pem"; chmod 600 "$G/mtls.key.pem"
  cp "$ST/pki/ca.pem" "$G/ca-roots.pem"
  printf 'current_context = "e2e"\n' > "$ST/goathome/config.toml"
  say "up: portal :18080, console :18081, identity :18082, router $( [ -f "$ST/router" ] && echo real || echo none); GOAT_PROFILE_DIR=$ST/goathome"
}

case "$MODE" in
  up) up ;;
  down) down; say "down" ;;
  *) echo "usage: $0 up|down <state-dir>" >&2; exit 2 ;;
esac
