#!/usr/bin/env bash
# Headless proof of the FULL #2 chain with the software test CAC (no real card):
# CAC cert -> nginx mTLS + EDIPI -> Dex -> Kratos federation -> native goat identity.
# This is what the browser run.sh does with a real CAC; here curl --cert stands in for
# the card so the whole chain is CI-provable. (Real-CAC + real-DoD-PKI = run.sh.)
set -euo pipefail
cd "$(dirname "$0")"
../cac-test-pki/gen-test-cac.sh 1234567890 >/dev/null
cp ../cac-test-pki/out/ca.crt dod-ca.pem                 # front-end trusts the test CA
openssl req -x509 -newkey rsa:2048 -nodes -keyout server.key -out server.crt -days 365 \
  -subj "/CN=cac-idp" -addext "subjectAltName=DNS:cac-idp" 2>/dev/null   # SAN=cac-idp

docker compose -f docker-compose.mtls-full.yml down -v >/dev/null 2>&1 || true
docker compose -f docker-compose.mtls-full.yml up -d 2>&1 | tail -1
trap 'docker compose -f docker-compose.mtls-full.yml down -v >/dev/null 2>&1' EXIT
echo "==> waiting for kratos"
for _i in $(seq 1 40); do curl -fsS http://localhost:4434/admin/health/ready >/dev/null 2>&1 && break || sleep 3; done

NET=$(docker compose -f docker-compose.mtls-full.yml ps --format '{{.Name}}' kratos | sed 's/-kratos-1//')_cac
docker run --rm --network "$NET" -v "$(cd ..;pwd)/cac-test-pki/out:/pki:ro" -v "$(pwd):/kc:ro" badouralix/curl-jq sh -c '
  J=/tmp/j; CRT="--cert /pki/cac.crt --key /pki/cac.key"; CA="--cacert /kc/server.crt"
  # 1) start Kratos oidc registration, submit provider=cac
  F=$(curl -fsS -c $J -b $J -H "Accept: application/json" http://kratos-idp:4433/self-service/registration/browser)
  A=$(echo "$F"|jq -r ".ui.action"); C=$(echo "$F"|jq -r ".ui.nodes[]|select(.attributes.name==\"csrf_token\").attributes.value")
  REDIR=$(curl -sS -c $J -b $J -H "Accept: application/json" -X POST "$A" -d "csrf_token=$C" -d "provider=cac" | jq -r ".redirect_browser_to // empty")
  [ -n "$REDIR" ] || { echo "  no redirect to CAC front-end"; exit 1; }
  # 2) follow the chain to the CAC front-end (present the test CAC) and back to Kratos
  curl -sS -c $J -b $J $CRT $CA -L --max-redirs 12 -o /dev/null "$REDIR" 2>/dev/null || true
' 2>&1 | sed "s/^/  /"

echo "==> goat identity minted from the CAC?"
IDS=$(curl -fsS http://localhost:4434/admin/identities)
echo "$IDS" | jq -e '[.[]|select(.metadata_public.external_subjects[]?.subject=="1234567890")]|length>=1' >/dev/null && {
  echo "$IDS" | jq '[.[]|select(.metadata_public.external_subjects[]?.subject=="1234567890")][0]|{id, email:.traits.email, groups:.metadata_public.groups, external_subjects:.metadata_public.external_subjects}'
  echo "  ==> FULL #2 CHAIN PROVEN: CAC cert -> nginx mTLS -> Dex -> Kratos -> native goat identity (anchor=Kratos UUID, EDIPI in metadata_public)"
} || { echo "  FAIL: no goat identity carrying the EDIPI"; echo "$IDS" | jq -r ".[].traits.email"; exit 1; }
