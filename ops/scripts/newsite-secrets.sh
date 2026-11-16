#!/usr/bin/env bash
# Mint missing site credentials into a secret store; write only references.
# Usage: newsite-secrets.sh <site-id> [--file <refs>] [--vault <vault>]
#        [--bootstrap-operator <handle>] [--dry-run | --spec-json | --cluster-json]
# Existing references and legacy assignments are never rotated (ADR 1186).
# Class-B values are for NEW fabrics; joining a cluster requires its existing
# NetBird, replication and Ory secrets, not a second genesis.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SITE="${1:-}"
case "$SITE" in
  ''|-*|*[!a-z0-9-]*) echo "usage: newsite-secrets.sh <site-id> [--file <refs>] [--vault <vault>] [--dry-run | --spec-json]" >&2; exit 2 ;;
esac
shift
S="$(printf '%s' "$SITE" | tr '[:lower:]-' '[:upper:]_')"
# name|generator|comment[|cluster]   (generator 'literal:<v>' emits a fixed value)
# `cluster` marks a value every site of the goatnet shares: a joining site takes the
# founder's (join-site-secrets.py, docs/design/join-node-site-secrets.md) and never mints one.
SPEC=$(cat <<EOF
${S}_PG_PASS|hex32|postgres superuser
${S}_SPIRE_DB_PASS|hex32|SPIRE datastore role on postgres-a (phase 9)
${S}_PORTAL_DB_PASS|hex32|portal registry role on postgres-a (phase 12)
${S}_ID_CONSOLE_DB_PASS|hex40|identity console Postgres role password (id_admin_console_store: postgres)
${S}_NB_ENCKEY|b64_32|netbird mgmt datastore key (Class-B: per-fabric)|cluster
${S}_BKP_KEY|b64_36|pgBackRest cipher pass
${S}_REPLICATOR_PASS|hex32|Spock/streaming replicator (Class-B: per-cluster)|cluster
${S}_RELAY_SECRET|b64_32|relay auth secret|cluster
${S}_SLOT_POOL_SALT|hex48|zenoh per-uuid slot pool salt (ADR 1025)
${S}_PORTAL_OPERATOR_API_KEY|b64_32|legacy portal service bearer
${S}_PORTAL_OPERATOR_API_KEYS|pair:|optional named bootstrap pair; requires --bootstrap-operator
${S}_KRATOS_CIPHER|hex32|MUST be exactly 32 chars|cluster
${S}_KRATOS_COOKIE|hex32|
${S}_HYDRA_SYSTEM|hex32||cluster
${S}_HYDRA_COOKIE|hex32|
${S}_UI_COOKIE_SECRET|hex32|kratos selfservice-ui
${S}_UI_CSRF_SECRET|hex32|kratos selfservice-ui
${S}_ORY_SHARED_NID|uuid|Ory shared network id|cluster
${S}_DASHBOARD_OIDC_CLIENT_SECRET|hex48|
${S}_DASHBOARD_OAUTH2_PROXY_CLIENT_SECRET|hex48|
${S}_DASHBOARD_OAUTH2_PROXY_COOKIE_SECRET|hex32|
${S}_ISITUP_OIDC_CLIENT_SECRET|hex48|
${S}_ISITUP_COOKIE_SECRET|hex32|
${S}_ID_ADMIN_CONSOLE_OAUTH2_PROXY_CLIENT_SECRET|hex48|
${S}_ID_ADMIN_CONSOLE_OAUTH2_PROXY_COOKIE_SECRET|hex32|
${S}_STETHOSCOPE_OIDC_CLIENT_SECRET|hex48|
${S}_COMPLIANCE_OIDC_CLIENT_SECRET|hex48|
${S}_COMPLIANCE_COOKIE_SECRET|hex32|
${S}_MICROSCOPE_OAUTH2_PROXY_CLIENT_SECRET|hex48|
${S}_MICROSCOPE_OAUTH2_PROXY_COOKIE_SECRET|hex32|
${S}_CONSISTENCY_CONSOLE_OAUTH2_PROXY_CLIENT_SECRET|hex48|
${S}_CONSISTENCY_CONSOLE_OAUTH2_PROXY_COOKIE_SECRET|hex32|
${S}_GOAT_MAP_OAUTH2_PROXY_CLIENT_SECRET|hex48|
${S}_CONSISTENCY_AGGREGATOR_OAUTH2_PROXY_CLIENT_SECRET|hex48|
${S}_CONSISTENCY_AGGREGATOR_OAUTH2_PROXY_COOKIE_SECRET|hex32|
${S}_MINIO_ROOT_USER|literal:${SITE}-minio-root|
${S}_MINIO_ROOT_PASSWORD|hex40|
${S}_PUBSUB_MIRROR_PORTAL_KEY|b64_32|Block 86 pubsub-state mirror service operator key (FORGEJO_PUBSUB_TOKEN is minted post-Forgejo by forgejo-mint-pubsub-bot.sh)
${S}_AUDIT_SUBSCRIBER_S3_ACCESS_KEY|literal:audit-subscriber|
${S}_AUDIT_SUBSCRIBER_S3_SECRET_KEY|hex40|
${S}_PORTAL_AUDIT_S3_ACCESS_KEY|literal:goat-portal-audit-read|
${S}_PORTAL_AUDIT_S3_SECRET_KEY|hex40|
${S}_STETHOSCOPE_AUDIT_S3_SECRET_KEY|hex40|utility tier
${S}_MICROSCOPE_AUDIT_S3_SECRET_KEY|hex40|utility tier
${S}_GRAFANA_OIDC_CLIENT_SECRET|hex48|utility tier
${S}_GRAFANA_ADMIN_PASS|hex32|utility tier
${S}_POSTGRES_EXPORTER_PASS|hex32|utility tier
EOF
)


printf '%s\n' "$SPEC" | python3 "$SCRIPT_DIR/newsite-secret-store.py" "$SITE" "$@"
