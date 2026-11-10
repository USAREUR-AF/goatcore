#!/usr/bin/env bash
# Prove toggle-switch PATH #1 (upstream_oidc) end-to-end, fully local.
# See README.md. Requires docker + docker compose + curl + jq.
set -euo pipefail
cd "$(dirname "$0")"

ADMIN=http://localhost:4434

echo "==> up"; docker compose up -d --wait 2>&1 | tail -2 || { docker compose up -d; }
echo "==> waiting for kratos admin"
for _i in $(seq 1 30); do curl -fsS "$ADMIN/admin/health/ready" >/dev/null 2>&1 && break || sleep 2; done
curl -fsS "$ADMIN/admin/health/ready" >/dev/null

# ---- CHECK A: an identity carries the external binding in metadata_public --------
echo "==> CHECK A: admin API mints identity with metadata_public.external_subjects"
EMAIL_A="checka-$(date +%s)@example.mil"
A=$(curl -fsS -X POST "$ADMIN/admin/identities" -H 'Content-Type: application/json' -d '{
  "schema_id": "default",
  "traits": { "email": "'"$EMAIL_A"'", "name": "Check A", "groups": ["ADMINS"] },
  "metadata_public": {
    "external_subjects": [{
      "issuer": "http://mock-oidc:8080/goatproof",
      "client_id": "goat-proof",
      "subject": "mock-pairwise-001",
      "scheme": "native",
      "credential_type": "oidc",
      "last_assurance": {"ial": "ial1", "aal": "aal2", "fal": "fal1", "acr": "aal2", "boundary_il": 0}
    }]
  }
}')
echo "$A" | jq -e '.metadata_public.external_subjects[0].subject == "mock-pairwise-001"' >/dev/null \
  && echo "   PASS: metadata_public.external_subjects accepted (id $(echo "$A" | jq -r .id))" \
  || { echo "   FAIL: metadata_public rejected"; echo "$A"; exit 1; }

# ---- CHECK B: federated login mints a native identity via the claim-map ---------
# The full OIDC redirect chain (kratos -> mock-oidc authorize -> callback) must speak
# ONE consistent set of hostnames, so we drive it from a curl container ON the compose
# network where `kratos` and `mock-oidc` resolve. Mock is non-interactive (auto-callback).
echo "==> CHECK B: federate via mock OIDC -> Kratos mints native anchor + external_subject"
NET=$(docker compose ps --format '{{.Name}}' kratos | head -1 | sed 's/-kratos-1//')_proof
docker run --rm --network "$NET" badouralix/curl-jq sh -c '
  set -e
  J=/tmp/j
  FLOW=$(curl -fsS -c $J -b $J -H "Accept: application/json" http://kratos:4433/self-service/registration/browser)
  ACTION=$(echo "$FLOW" | jq -r ".ui.action")
  CSRF=$(echo "$FLOW" | jq -r ".ui.nodes[] | select(.attributes.name==\"csrf_token\") | .attributes.value")
  echo "  action=$ACTION"
  # Kratos browser-flow oidc submit returns 422 + {redirect_browser_to} (the mock authorize URL).
  RESP=$(curl -sS -c $J -b $J -H "Accept: application/json" -X POST "$ACTION" \
    -d "csrf_token=$CSRF" -d "provider=goatproof")
  REDIR=$(echo "$RESP" | jq -r ".redirect_browser_to // empty")
  echo "  redirect_browser_to=$REDIR"
  [ -n "$REDIR" ] || { echo "  no redirect; resp=$RESP"; exit 0; }
  # Follow the browser redirect: mock authorize (non-interactive) -> kratos callback -> mint.
  curl -sS -c $J -b $J -L "$REDIR" -o /dev/null -w "  followed chain, final_http=%{http_code}\n" || true
' 2>&1 | sed 's/^/  /'

# The oidc mapper sets traits (email/name/groups) AND metadata_public.external_subjects
# in one shot at registration. Assert the native identity was minted from the federated
# login with the fail-closed role mapping AND the external binding in metadata_public.
FED=$(curl -fsS "$ADMIN/admin/identities" | jq '[.[] | select(.traits.email=="person-22@example.org")][0]')
if [ "$(echo "$FED" | jq -r '.id // empty')" = "" ]; then
  echo "   FAIL: federated identity not minted"; exit 1
fi
OK=$(echo "$FED" | jq -e '
  .traits.groups==["ADMINS"] and
  .metadata_public.external_subjects[0].subject=="mock-pairwise-001" and
  .metadata_public.external_subjects[0].scheme=="native" and
  .metadata_public.external_subjects[0].last_assurance.aal=="aal2"' >/dev/null && echo yes || echo no)
echo "$FED" | jq '{id, email: .traits.email, groups: .traits.groups, external_subjects: .metadata_public.external_subjects}'
if [ "$OK" = "yes" ]; then
  echo "   ==> CHECK B GREEN: federated login -> native anchor + fail-closed role (ADMINS) + external binding + assurance"
else
  echo "   FAIL: minted identity missing expected role/binding/assurance"; exit 1
fi
echo "==> done. 'docker compose down -v' to clean up."
