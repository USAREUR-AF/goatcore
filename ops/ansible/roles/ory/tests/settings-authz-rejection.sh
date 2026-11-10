#!/usr/bin/env bash
# settings-authz-rejection.sh — the load-bearing test for F-572 / ADR 1188.
#
# Every other test in this change mocks Kratos. This one stands up a REAL Kratos
# (v1.3.1, matching the ory-consent image FROM) with our identity schema and
# drives the self-service settings flow as a signed-in user, trying to
# self-grant a login group. It asserts:
#
#   ory_authz_in_metadata = true   →  the settings submit carrying traits.groups
#                                      is REJECTED (400). The escalation is closed.
#   ory_authz_in_metadata = false  →  the same submit is ACCEPTED (200). This is
#                                      the vulnerable pre-flag state — proof that
#                                      flipping the flag is what closes the hole.
#
# Minimal by design: Kratos + in-memory SQLite, no Hydra/Spock/courier — so it
# boots in seconds and runs on any Docker host (multi-arch image, incl. Apple
# Silicon). Needs only Docker + curl + python3.
#
#   ops/ansible/roles/ory/tests/settings-authz-rejection.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../../../../.." && pwd)"
TEMPLATE="$REPO_ROOT/ops/ansible/roles/ory/templates/ory/identity-schema.json.j2"
KRATOS_IMAGE="${KRATOS_IMAGE:-oryd/kratos:v1.3.1}"
PUB=4455; ADM=4456
WORK="$(mktemp -d -t ory-authz-test-XXXXXX)"
CID=""
cleanup() { [ -n "$CID" ] && docker rm -f "$CID" >/dev/null 2>&1 || true; rm -rf "$WORK"; }
trap cleanup EXIT

# Render the identity schema for a given flag state. Mirrors the two Jinja
# conditionals in the template (ory_authz_in_metadata, ory_site_vendor_trait_compat);
# vendor compat is false here, as on a goatnet that never carried the vendor trait.
render_schema() {  # $1 = authz_in_metadata (true|false)  -> stdout JSON
  AUTHZ="$1" python3 - "$TEMPLATE" <<'PY'
import json, os, sys
src = open(sys.argv[1]).read()
authz = os.environ["AUTHZ"] == "true"
vendor = False  # ory_site_vendor_trait_compat off
base = src.replace("{{ site_dns_zone }}", "test.local").replace("{{ site_id }}", "test")
base = base.replace("{{ ory_brand_name | default('goat') }}", "goat")
out, stack = [], []
for ln in base.splitlines(keepends=True):
    st = ln.strip()
    if st.startswith("{% if not ory_authz_in_metadata"): stack.append(not authz); continue
    if st.startswith("{% if ory_site_vendor_trait_compat"): stack.append(vendor); continue
    if st == "{% endif %}": stack.pop(); continue
    if all(stack): out.append(ln)
txt = "".join(out)
json.loads(txt)  # fail loudly if the render is not valid JSON
sys.stdout.write(txt)
PY
}

write_config() {
  # dsn: memory → Kratos boots an in-memory SQLite and auto-migrates in --dev.
  # methods live UNDER selfservice (a top-level `methods` is rejected by the
  # config schema). Kratos listens on the image defaults 4433/4434; base_url just
  # advertises the host-mapped ports so the flow `action` URLs are host-reachable.
  cat > "$WORK/kratos.yml" <<YAML
dsn: memory
serve:
  public: { base_url: "http://127.0.0.1:$PUB/" }
  admin:  { base_url: "http://127.0.0.1:$ADM/" }
identity:
  default_schema_id: default
  schemas: [{ id: default, url: "file:///etc/config/identity.schema.json" }]
selfservice:
  default_browser_return_url: "http://127.0.0.1:$PUB/"
  flows:
    login:    { ui_url: "http://127.0.0.1:$PUB/login" }
    settings: { ui_url: "http://127.0.0.1:$PUB/settings", privileged_session_max_age: 24h }
  methods:
    password: { enabled: true, config: { min_password_length: 8, haveibeenpwned_enabled: false } }
    webauthn: { enabled: true, config: { passwordless: false, rp: { display_name: t, id: 127.0.0.1, origins: ["http://127.0.0.1:$PUB"] } } }
    totp:     { enabled: true, config: { issuer: t } }
log: { level: error }
YAML
}

boot_kratos() {  # $1 = schema json path
  CID=$(docker run -d --rm \
    -p "$PUB:4433" -p "$ADM:4434" \
    -v "$WORK/kratos.yml:/etc/config/kratos.yml:ro" \
    -v "$1:/etc/config/identity.schema.json:ro" \
    "$KRATOS_IMAGE" serve --dev --config /etc/config/kratos.yml)
  for _ in $(seq 1 60); do
    curl -fsS "http://127.0.0.1:$ADM/admin/health/ready" >/dev/null 2>&1 && return 0
    sleep 1
  done
  echo "FATAL: Kratos did not become ready"; docker logs "$CID" 2>&1 | tail -20; return 1
}

# Returns the HTTP status of a self-service settings submit that tries to add a
# login group to traits, for the currently-running Kratos.
attempt_self_grant() {  # $1 = email  $2 = password  -> echoes HTTP status
  # 1) API login → session token
  local login_flow token settings_flow action
  login_flow=$(curl -fsS "http://127.0.0.1:$PUB/self-service/login/api")
  action=$(python3 -c 'import json,sys;print(json.loads(sys.argv[1])["ui"]["action"])' "$login_flow")
  token=$(curl -fsS -X POST "$action" -H 'Content-Type: application/json' \
            -d "{\"method\":\"password\",\"identifier\":\"$1\",\"password\":\"$2\"}" \
          | python3 -c 'import json,sys;print(json.load(sys.stdin).get("session_token",""))')
  [ -n "$token" ] || { echo "LOGIN_FAILED"; return 0; }
  # 2) settings flow (authenticated) → try to inject groups into traits
  settings_flow=$(curl -fsS "http://127.0.0.1:$PUB/self-service/settings/api" -H "Authorization: Bearer $token")
  action=$(python3 -c 'import json,sys;print(json.loads(sys.argv[1])["ui"]["action"])' "$settings_flow")
  curl -s -o "$WORK/resp.json" -w '%{http_code}' -X POST "$action" \
    -H "Authorization: Bearer $token" -H 'Content-Type: application/json' \
    -d "{\"method\":\"profile\",\"traits\":{\"email\":\"$1\",\"name\":\"Tester\",\"groups\":[\"PLATFORM-ADMIN\"]}}"
}

run_case() {  # $1 = authz flag  $2 = expected status
  local schema="$WORK/schema-$1.json" email="user-$1@test.local" pw="correcthorse12"
  render_schema "$1" > "$schema"
  write_config
  boot_kratos "$schema"
  curl -fsS -X POST "http://127.0.0.1:$ADM/admin/identities" -H 'Content-Type: application/json' \
    -d "{\"schema_id\":\"default\",\"state\":\"active\",\"traits\":{\"email\":\"$email\",\"name\":\"Tester\"},\"credentials\":{\"password\":{\"config\":{\"password\":\"$pw\"}}}}" \
    >/dev/null
  local got; got=$(attempt_self_grant "$email" "$pw")
  docker rm -f "$CID" >/dev/null 2>&1; CID=""
  if [ "$got" = "$2" ]; then
    echo "PASS  ory_authz_in_metadata=$1 : self-grant settings submit → HTTP $got (expected $2)"
  else
    echo "FAIL  ory_authz_in_metadata=$1 : self-grant settings submit → HTTP $got (expected $2)"
    echo "      response: $(cat "$WORK/resp.json" 2>/dev/null | head -c 400)"
    return 1
  fi
}

command -v docker >/dev/null || { echo "FATAL: docker not found"; exit 2; }
docker info >/dev/null 2>&1 || { echo "FATAL: docker daemon not reachable (start Docker Desktop)"; exit 2; }

echo "== F-572 / ADR 1188 live settings-flow rejection test =="
fail=0
run_case false 200 || fail=1   # vulnerable pre-flag: self-grant ACCEPTED
run_case true  400 || fail=1    # fixed: self-grant REJECTED by the schema
if [ "$fail" = 0 ]; then
  echo "ALL PASS — flipping ory_authz_in_metadata closes the self-service group self-grant."
else
  echo "TEST FAILED"; exit 1
fi
