#!/usr/bin/env bash
# Stands in for `goat admin onboard <handle> --native ... --env <env> --repo-root <root>`
# inside the partition harness (console.sh). It makes the two writes the native
# mint makes, at the runner's own site ($HARNESS_SITE): a NetBird setup key named
# for the handle (inventory N1) and a Kratos identity for the manifest's email,
# owned by that site (ADR 1205 D5). An identity that already exists at this site
# is reused, as the native mint's Ory leg is idempotent. It prints the summary
# line the runner parses ("setup key: <key>").
set -euo pipefail
[[ "${1:-}" == admin && "${2:-}" == onboard ]] || { echo "error: goat-shim only does 'admin onboard'" >&2; exit 2; }
handle="$3"; shift 3
env="" root=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --env) env="$2"; shift 2 ;;
    --repo-root) root="$2"; shift 2 ;;
    *) shift ;;
  esac
done
site="${HARNESS_SITE:?}"; : "${HARNESS_PFX:?}"
manifest="$root/ops/operators/$env/$handle.yml"
email=$(sed -nE 's/^[[:space:]]*email:[[:space:]]*"?([^"[:space:]]+)"?.*/\1/p' "$manifest" | head -1)
[[ -n "$email" ]] || { echo "error: no identity.email in $manifest" >&2; exit 1; }

id=$(python3 -c 'import secrets; print(secrets.token_hex(10))')
key=$(psql -h "$HARNESS_PFX-$site" -U postgres -d netbird -v ON_ERROR_STOP=1 -tAXq -c "
  INSERT INTO setup_keys (id, account_id, key, key_secret, name, type, created_at, expires_at, updated_at,
                          revoked, used_times, auto_groups, usage_limit, ephemeral, allow_extra_dns_labels)
  SELECT '$id', id, encode(sha256(random()::text::bytea), 'base64'), 'HARN****', '$handle', 'one-off',
         now(), now() + interval '1 day', now(), false, 0, '[]', 1, false, false
  FROM accounts LIMIT 1
  RETURNING key;")
[[ -n "$key" ]] || { echo "error: NetBird setup key not created at site $site" >&2; exit 1; }

body=$(python3 -c 'import json,sys; print(json.dumps({"schema_id":"default","traits":{"email":sys.argv[1],"name":sys.argv[2]},
  "metadata_admin":{"owner_site":"site_"+sys.argv[3]}}))' "$email" "$handle" "$site")
code=$(curl -s -o /tmp/kratos.out -w '%{http_code}' -X POST "http://$HARNESS_PFX-kratos-$site:4434/admin/identities" \
  -H 'Content-Type: application/json' -d "$body")
case "$code" in
  201) ;;
  409) echo "  identity:     $email already exists at site_$site (reused)" ;;
  *) echo "error: Kratos POST /admin/identities at site $site: HTTP $code $(head -c 200 /tmp/kratos.out)" >&2; exit 1 ;;
esac
echo "native onboard of $handle at site_$site"
echo "  setup key:    $key  (key \`$handle\`)"
