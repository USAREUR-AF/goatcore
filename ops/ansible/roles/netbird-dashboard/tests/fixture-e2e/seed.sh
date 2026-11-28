#!/usr/bin/env bash
# Seed the fixture stack:
#   - Generate a multi-SAN self-signed leaf cert covering the four
#     mgmt-stack hostnames.
#   - Render Traefik dynamic config pointing at the cert.
#   - Render Kratos identity-schema.json + kratos.yaml + hydra.yaml +
#     mgmt management.json (placeholders + secrets injected).
#   - Provision the Hydra OIDC client `netbird-dashboard` with the
#     four load-bearing fields (skip_consent, access_token_strategy,
#     redirect_uris, token_endpoint_auth_method).
#   - Pre-create a Kratos identity for the test operator.
#   - Pre-create the netbird-mgmt account (so /api/instance reports
#     setup_required=false on first dashboard hit).
#
# Idempotent: re-runs OK. Run AFTER `docker compose up -d --wait`.
#
# Reads E2E_OPERATOR_EMAIL + E2E_OPERATOR_PASSWORD from env (defaults
# in run.sh).

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
CONF_DIR="$HERE/conf"
TLS_DIR="$CONF_DIR/tls"
mkdir -p "$CONF_DIR" "$TLS_DIR"

EMAIL="${E2E_OPERATOR_EMAIL:?set E2E_OPERATOR_EMAIL}"
PASSWORD="${E2E_OPERATOR_PASSWORD:?set E2E_OPERATOR_PASSWORD}"

# F-085 ordering: seed.sh has two phases.
#   SEED_PHASE=config  : write config files + TLS cert (BEFORE compose up).
#   SEED_PHASE=runtime : provision Hydra OIDC client + Kratos identity
#                        (AFTER compose up; talks to running services).
#   SEED_PHASE unset   : run both (legacy callers / dev-laptop convenience).
SEED_PHASE="${SEED_PHASE:-all}"

if [[ "$SEED_PHASE" == "config" || "$SEED_PHASE" == "all" ]]; then

# ---- 1. multi-SAN self-signed cert -----------------------------------------
if [[ ! -f "$TLS_DIR/cert.pem" ]]; then
  echo "[seed] generating multi-SAN cert..."
  cat > /tmp/e2e-leaf.cnf <<EOF
[req]
distinguished_name = dn
prompt = no
req_extensions = v3_req
[dn]
CN = dashboard.localhost.test
[v3_req]
basicConstraints = critical, CA:FALSE
keyUsage = critical, digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
subjectAltName = @alt_names
[alt_names]
DNS.1 = dashboard.localhost.test
DNS.2 = mgmt.localhost.test
DNS.3 = oauth2.localhost.test
DNS.4 = auth.localhost.test
DNS.5 = localhost
EOF
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 \
    -keyout "$TLS_DIR/key.pem" -out "$TLS_DIR/cert.pem" \
    -days 30 -sha256 -nodes -config /tmp/e2e-leaf.cnf -extensions v3_req
  rm -f /tmp/e2e-leaf.cnf
fi

# ---- 2. Traefik dynamic config ---------------------------------------------
# Mirrors the production traefik role's rendered shape (see
# ops/ansible/roles/traefik/templates/dynamic.yml.j2). The
# `dashboard-root-to-access-control` middleware MUST be attached
# to the dashboard router here so the e2e harness exercises the
# same code path that production runs against — F-080 closure.
cat > "$CONF_DIR/traefik-dynamic.yml" <<'EOF'
tls:
  options:
    default:
      minVersion: VersionTLS12
  certificates:
    - certFile: /etc/traefik/tls/cert.pem
      keyFile: /etc/traefik/tls/key.pem

http:
  middlewares:
    dashboard-root-to-access-control:
      redirectRegex:
        regex: "^(https?://[^/]+)?/?$"
        replacement: "${1}/access-control"
        permanent: false
  routers:
    dashboard:
      rule: "Host(`dashboard.localhost.test`)"
      entryPoints: [websecure]
      service: dashboard
      middlewares:
        - dashboard-root-to-access-control
      tls: {}
    mgmt:
      rule: "Host(`mgmt.localhost.test`)"
      entryPoints: [websecure]
      service: mgmt
      tls: {}
    hydra:
      rule: "Host(`oauth2.localhost.test`)"
      entryPoints: [websecure]
      service: hydra
      tls: {}
    kratos:
      rule: "Host(`auth.localhost.test`) && (PathPrefix(`/sessions`) || PathPrefix(`/self-service`) || PathPrefix(`/.well-known`) || PathPrefix(`/schemas`) || PathPrefix(`/health`))"
      priority: 100
      entryPoints: [websecure]
      service: kratos
      tls: {}
    selfservice-ui:
      rule: "Host(`auth.localhost.test`)"
      priority: 10
      entryPoints: [websecure]
      service: selfservice-ui
      tls: {}
  services:
    dashboard:
      loadBalancer:
        servers: [{ url: "http://dashboard:80" }]
    mgmt:
      loadBalancer:
        servers: [{ url: "http://netbird-mgmt:80" }]
    hydra:
      loadBalancer:
        servers: [{ url: "http://hydra:4444" }]
    kratos:
      loadBalancer:
        servers: [{ url: "http://kratos:4433" }]
    selfservice-ui:
      loadBalancer:
        servers: [{ url: "http://selfservice-ui:3000" }]
EOF

# ---- 3. Kratos identity schema + kratos.yaml + hydra.yaml + mgmt config ----
cat > "$CONF_DIR/identity-schema.json" <<'EOF'
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "type": "object",
  "properties": {
    "traits": {
      "type": "object",
      "properties": {
        "email": {
          "type": "string", "format": "email",
          "ory.sh/kratos": {
            "credentials": {
              "password": { "identifier": true },
              "totp": { "account_name": true },
              "webauthn": { "identifier": true }
            },
            "verification": { "via": "email" },
            "recovery": { "via": "email" }
          }
        }
      },
      "required": ["email"],
      "additionalProperties": false
    }
  }
}
EOF

cat > "$CONF_DIR/kratos.yaml" <<EOF
serve:
  public:
    base_url: https://auth.localhost.test/
    cors:
      enabled: true
      allowed_origins: ["https://dashboard.localhost.test", "https://oauth2.localhost.test", "https://auth.localhost.test"]
      allowed_methods: [POST, GET, PUT, PATCH, DELETE, OPTIONS]
      allowed_headers: [Authorization, Cookie, Content-Type, X-Session-Token]
      allow_credentials: true
  admin:
    base_url: http://kratos:4434/

selfservice:
  default_browser_return_url: https://dashboard.localhost.test/
  allowed_return_urls:
    - https://dashboard.localhost.test
    - https://oauth2.localhost.test
  methods:
    password:
      enabled: true
  flows:
    error: { ui_url: https://auth.localhost.test/error }
    settings: { ui_url: https://auth.localhost.test/settings, privileged_session_max_age: 15m }
    recovery: { enabled: false }
    verification: { enabled: false }
    logout: { after: { default_browser_return_url: https://dashboard.localhost.test/ } }
    login:
      ui_url: https://auth.localhost.test/login
      lifespan: 10m
    registration:
      enabled: true
      ui_url: https://auth.localhost.test/registration
      lifespan: 10m
      after:
        password:
          hooks:
            - hook: session

oauth2_provider:
  url: http://hydra:4445

identity:
  default_schema_id: default
  schemas:
    - id: default
      url: file:///etc/config/identity-schema.json

courier:
  smtp:
    connection_uri: smtps://test:test@localhost:1025/
EOF

cat > "$CONF_DIR/hydra.yaml" <<EOF
serve:
  cookies:
    same_site_mode: Lax
  public:
    cors:
      enabled: true
      allowed_origins: ["https://dashboard.localhost.test"]
      allowed_methods: [POST, GET, PUT, DELETE, OPTIONS]

urls:
  self:
    issuer: https://oauth2.localhost.test/
  consent: https://auth.localhost.test/consent
  login: https://auth.localhost.test/login
  logout: https://auth.localhost.test/logout

oidc:
  subject_identifiers:
    supported_types: [public]
    pairwise:
      salt: e2e-pairwise-salt-fixed-32xxxxxxxxxxx

ttl:
  login_consent_request: 10m
  access_token: 1h
  refresh_token: 720h
  id_token: 1h
  auth_code: 10m

strategies:
  access_token: jwt
EOF

cat > "$CONF_DIR/management.json" <<EOF
{
  "Stuns": [{"Proto":"udp","URI":"stun:localhost:3478"}],
  "TURNConfig": {"Turns":[],"CredentialsTTL":"12h","Secret":"none"},
  "Signal": {"Proto":"http","URI":"localhost:80"},
  "Datadir": "/var/lib/netbird",
  "DataStoreEncryptionKey": "2KktuRrqu+C9aImt77zNPEdTTqe7iVQ3I2RrWQtAkkM=",
  "HttpConfig": {
    "Address": "0.0.0.0:80",
    "AuthAudience": "netbird-dashboard",
    "AuthIssuer": "https://oauth2.localhost.test/",
    "AuthKeysLocation": "http://hydra:4444/.well-known/jwks.json",
    "AuthUserIDClaim": "sub",
    "IdpSignKeyRefreshEnabled": true
  },
  "IdpManagerConfig": {"ManagerType":"none"},
  "ReverseZone": "in-addr.arpa.",
  "StoreConfig": {"Engine":"postgres"}
}
EOF
# NOTE: OIDCConfigEndpoint intentionally OMITTED.
# When set, netbird-mgmt fetches the discovery doc at boot and OVERRIDES
# the locally-configured AuthKeysLocation with the `jwks_uri` value from
# Hydra's discovery response. Hydra publishes `https://oauth2.localhost.test/
# .well-known/jwks.json` (the public URL that matches its issuer) — but
# `oauth2.localhost.test` is only mapped to 127.0.0.1 on the HOST (via
# /etc/hosts inside the CI runner), NOT inside the netbird-mgmt container.
# The override therefore points at an unreachable URL; every JWT validation
# fails with "getPublicKey error: unable to find appropriate key" → 401.
# Without OIDCConfigEndpoint, no discovery fetch happens; AuthKeysLocation
# stays at the working internal http://hydra:4444/.well-known/jwks.json.
# AuthIssuer must still match the `iss` claim in the JWT, which is what
# Hydra signs with — kept at https://oauth2.localhost.test/.

cat > "$CONF_DIR/init-databases.sql" <<'EOF'
CREATE DATABASE kratos;
CREATE DATABASE hydra;
CREATE DATABASE netbird;
EOF

echo "[seed/config] config files + TLS cert written."
fi  # end SEED_PHASE=config block

if [[ "$SEED_PHASE" == "runtime" || "$SEED_PHASE" == "all" ]]; then

# ---- 4. Wait for Hydra admin to be reachable, then provision OIDC client ---
echo "[seed] waiting for Hydra admin API..."
for _ in {1..30}; do
  if docker compose exec -T hydra wget -q -O- http://localhost:4445/health/ready | grep -q ok; then
    break
  fi
  sleep 1
done

echo "[seed] upserting Hydra OIDC client netbird-dashboard..."
# Hydra v2.3.0 CLI flag is `--id` (not `--client-id`). The earlier
# command silently failed because of this typo + the `|| true` pipe
# swallowed the error → Playwright then drove the dance against a
# non-existent client → invalid_client / unknown client. Drop `|| true`
# so future flag drift fails the seed loudly instead of cascading into
# the OIDC flow.
docker compose exec -T hydra hydra create oauth2-client \
  --endpoint http://localhost:4445 \
  --name "netbird-dashboard (e2e)" \
  --grant-type authorization_code,refresh_token \
  --response-type code \
  --scope "openid profile email offline_access" \
  --redirect-uri https://dashboard.localhost.test/peers \
  --redirect-uri https://dashboard.localhost.test/access-control \
  --redirect-uri https://dashboard.localhost.test/silent-auth \
  --post-logout-callback https://dashboard.localhost.test/ \
  --audience netbird-dashboard \
  --secret e2e-dashboard-client-secret-fixed \
  --token-endpoint-auth-method client_secret_post \
  --skip-consent \
  --access-token-strategy jwt \
  --id netbird-dashboard 2>&1 | grep -v "already exists" || \
  docker compose exec -T hydra hydra get oauth2-client --endpoint http://localhost:4445 netbird-dashboard 2>&1 | grep -q netbird-dashboard

# ---- 5. Pre-create Kratos identity for the test operator -------------------
# Kratos image (oryd/kratos:v26.2.0) ships wget but NOT curl. The earlier
# `curl -sS -X POST` from inside the container failed immediately with
# "executable file not found"; `|| echo "..."` swallowed the error so
# the identity never got created. Login then failed with "invalid
# credentials" because no password credential existed in the DB.
#
# Use busybox wget's --post-data + --header from inside the container.
# Kratos admin :4434 isn't published host-side, so this stays inside the
# compose network. Assert the response contains an `id` field (success)
# or `409` / `already exists` (idempotent re-run); fail loudly otherwise.
echo "[seed] pre-creating Kratos identity ${EMAIL}..."
KRATOS_SEED_PAYLOAD=$(cat <<JSON
{"schema_id":"default","traits":{"email":"${EMAIL}"},"credentials":{"password":{"config":{"password":"${PASSWORD}"}}}}
JSON
)
docker compose exec -T kratos wget -qO- \
  --header="Content-Type: application/json" \
  --post-data="${KRATOS_SEED_PAYLOAD}" \
  http://localhost:4434/admin/identities \
  > /tmp/seed-kratos-identity.log 2>&1 || true
if ! grep -qE '"id"|"already exists"|409|"code":409' /tmp/seed-kratos-identity.log; then
  echo "[seed] FATAL: Kratos identity create did not return an id" >&2
  echo "[seed] kratos response:" >&2
  cat /tmp/seed-kratos-identity.log >&2
  exit 7
fi
echo "[seed] kratos identity create OK"

echo "[seed/runtime] OIDC client + Kratos identity seeded."
fi  # end SEED_PHASE=runtime block

echo "[seed] done."
