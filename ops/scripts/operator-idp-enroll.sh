#!/usr/bin/env bash
# Block 21H.2 — initial operator enrollment for the operator IdP per
# ADR 0842. POSTs to the per-site Kratos admin API to create an
# operator identity, then generates a recovery link the new operator
# uses to complete WebAuthn enrollment on first login.
#
# Runs ON Orin-C (where the operator-IdP Kratos lives) — operator SSHes
# in and invokes this script. The Kratos admin port (default 4436) is
# bound to localhost only; this script reaches it via loopback.
#
# Self-service registration is OFF in operator-idp-kratos config — this
# script is the ONLY way to create an operator identity. That's the
# point: anyone able to hit the public Kratos UI must NOT be able to
# mint themselves an obs-admin role.
#
# Usage:
#   sudo operator-idp-enroll.sh \
#     --email person-17@example.org \
#     --name "Alice Admin" \
#     --groups obs-admin
#
#   sudo operator-idp-enroll.sh \
#     --email person-23@example.org \
#     --name "Bob SRE" \
#     --groups grafana-admin,minio-readonly,forgejo-user
#
# Exit codes:
#   0 — identity created, recovery link printed
#   1 — usage / argument error
#   2 — Kratos admin API not reachable
#   3 — duplicate identity (email already exists)
#   4 — group string not in schema enum

set -euo pipefail

KRATOS_ADMIN_HOST="${KRATOS_ADMIN_HOST:-127.0.0.1}"
KRATOS_ADMIN_PORT="${KRATOS_ADMIN_PORT:-4436}"
KRATOS_ADMIN="http://${KRATOS_ADMIN_HOST}:${KRATOS_ADMIN_PORT}"

# Allowed group strings — must match the enum in
# operator-idp-kratos/templates/identity-schema.json.j2. Drift here =
# silent rejection by Kratos schema validation.
ALLOWED_GROUPS=(
  obs-admin
  obs-readonly
  grafana-admin
  grafana-readonly
  minio-admin
  minio-readonly
  openobserve-admin
  openobserve-readonly
  forgejo-admin
  forgejo-user
  prometheus-readonly
  alertmanager-admin
  ollama-user
)

usage() {
  cat <<USAGE
Usage: $0 --email <email> --name <name> --groups <g1>[,<g2>,...]

Creates an operator identity in the per-site operator IdP and emits a
one-time recovery link the new operator uses to enroll their FIDO2
token (WebAuthn-only credential method per ADR 0842).

Required:
  --email <email>     Operator email (used as login identifier).
  --name <name>       Display name (e.g., "Alice Admin").
  --groups <list>     Comma-separated authorization groups.

Allowed groups (must match operator-idp-kratos schema):
  ${ALLOWED_GROUPS[*]}

Special groups:
  obs-admin       Admin on every observability consumer.
  obs-readonly    Read-only on every observability consumer.
  <svc>-admin     Admin scoped to one consumer (Grafana, MinIO, etc.).
  <svc>-readonly  Read-only scoped to one consumer.
USAGE
  exit 1
}

EMAIL=""
NAME=""
GROUPS_RAW=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --email)  EMAIL="$2";       shift 2 ;;
    --name)   NAME="$2";        shift 2 ;;
    --groups) GROUPS_RAW="$2";  shift 2 ;;
    -h|--help) usage ;;
    *) echo "unknown flag: $1" >&2; usage ;;
  esac
done

[[ -z "$EMAIL" || -z "$NAME" || -z "$GROUPS_RAW" ]] && usage

# Validate every group is in the allow-list. Catches typos early; Kratos
# would reject too, but with a less clear error.
IFS=',' read -r -a GROUPS_ARR <<< "$GROUPS_RAW"
for g in "${GROUPS_ARR[@]}"; do
  found=0
  for allowed in "${ALLOWED_GROUPS[@]}"; do
    [[ "$g" == "$allowed" ]] && { found=1; break; }
  done
  if [[ $found -eq 0 ]]; then
    echo "error: group '$g' not in schema enum" >&2
    echo "       allowed: ${ALLOWED_GROUPS[*]}" >&2
    exit 4
  fi
done

# Reachability check before we do any state-modifying work.
if ! curl -fsS -o /dev/null --max-time 3 "${KRATOS_ADMIN}/admin/health/ready"; then
  echo "error: Kratos admin API at ${KRATOS_ADMIN} not reachable" >&2
  echo "       expected operator-idp-kratos to be running on this host." >&2
  exit 2
fi

# Build the JSON payload. jq is the safe way to construct JSON with
# user-controlled strings (no shell-quoting bugs).
PAYLOAD=$(jq -n \
  --arg email "$EMAIL" \
  --arg name  "$NAME" \
  --argjson groups "$(printf '%s\n' "${GROUPS_ARR[@]}" | jq -R . | jq -s .)" \
  '{schema_id: "operator", traits: {email: $email, name: $name, groups: $groups}}')

# Create the identity. Kratos 409s on duplicate email.
HTTP_OUT=$(mktemp)
trap 'rm -f "$HTTP_OUT"' EXIT
HTTP_CODE=$(curl -sS -o "$HTTP_OUT" -w '%{http_code}' \
  -X POST "${KRATOS_ADMIN}/admin/identities" \
  -H 'Content-Type: application/json' \
  -d "$PAYLOAD")

case "$HTTP_CODE" in
  201) ;; # success
  409)
    echo "error: identity already exists for email=${EMAIL}" >&2
    echo "       to update groups, use:" >&2
    echo "         curl -X PATCH ${KRATOS_ADMIN}/admin/identities/<id> ..." >&2
    exit 3
    ;;
  *)
    echo "error: Kratos returned HTTP ${HTTP_CODE}" >&2
    cat "$HTTP_OUT" >&2
    exit 2
    ;;
esac

IDENTITY_ID=$(jq -r '.id' "$HTTP_OUT")

# Generate a one-time recovery link and code. The new operator opens the
# link in their browser, enters the code, and lands in the Kratos settings
# flow where they enroll their FIDO2 token. Kratos v1.3.1 route:
# POST /admin/recovery/code (201). Expires in 1h by default.
RECOVERY_PAYLOAD=$(jq -n --arg id "$IDENTITY_ID" '{identity_id: $id}')
RECOVERY_OUT=$(mktemp)
trap 'rm -f "$HTTP_OUT" "$RECOVERY_OUT"' EXIT
RECOVERY_CODE=$(curl -sS -o "$RECOVERY_OUT" -w '%{http_code}' \
  -X POST "${KRATOS_ADMIN}/admin/recovery/code" \
  -H 'Content-Type: application/json' \
  -d "$RECOVERY_PAYLOAD")

if [[ "$RECOVERY_CODE" != "201" ]]; then
  echo "warning: identity created (id=${IDENTITY_ID}) but recovery link failed (HTTP ${RECOVERY_CODE})" >&2
  cat "$RECOVERY_OUT" >&2
  echo "         operator can re-trigger via:" >&2
  echo "           curl -X POST ${KRATOS_ADMIN}/admin/recovery/code -H 'Content-Type: application/json' -d '{\"identity_id\":\"${IDENTITY_ID}\"}'" >&2
  exit 0
fi

RECOVERY_LINK=$(jq -r '.recovery_link' "$RECOVERY_OUT")
RECOVERY_ONE_TIME_CODE=$(jq -r '.recovery_code' "$RECOVERY_OUT")
EXPIRES_AT=$(jq -r '.expires_at' "$RECOVERY_OUT")

cat <<DONE
========================================================================
 Operator identity created
========================================================================
 ID:       ${IDENTITY_ID}
 Email:    ${EMAIL}
 Name:     ${NAME}
 Groups:   ${GROUPS_RAW}
 Expires:  ${EXPIRES_AT}

 Recovery link and code (share with the new operator out-of-band —
 Matrix DM / phone call / in-person; do NOT email):

 ${RECOVERY_LINK}
 Code: ${RECOVERY_ONE_TIME_CODE}

 New-operator workflow:
   1. Open the link in a browser on the same network as the operator
      IdP (wg-cp0 mesh).
   2. Enter the code. Kratos lands them in the settings flow.
      Click "Set up Passkey."
   3. Plug in FIDO2 token, follow the WebAuthn prompts.
   4. Done. Operator can now log into any operator-IdP-fronted
      consumer (Grafana, MinIO, OpenObserve, Forgejo) using their
      token.
========================================================================
DONE
