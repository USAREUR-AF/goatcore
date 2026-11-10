#!/usr/bin/env bash
# ops/ansible/scripts/revoke-setup-key.sh — revoke one Netbird setup key
# with audit trail. Cascade options for suspected-compromise response.
#
# Usage:
#   revoke-setup-key.sh --key-name vega-admin-000012
#                       [--reason "rotation" | "compromise" | "expired"]
#                       [--cascade-from-operator USER --since RFC3339]
#                       --ssm-instance i-... --ssm-region eu-central-1
#
# AWS_VAULT_PROFILE names the aws-vault profile of the account that holds the
# instance.
#
# Modes:
#   single:  revoke exactly the named key
#   cascade: revoke every unrevoked key created by a named operator since
#            a given timestamp (used in suspected-compromise response)
#
set -euo pipefail

KEY_NAME=""; REASON="rotation"; CASCADE_OP=""; CASCADE_SINCE=""
SSM_INSTANCE=""; SSM_REGION=""

while [ $# -gt 0 ]; do
  case "$1" in
    --key-name)              KEY_NAME=$2; shift 2;;
    --reason)                REASON=$2; shift 2;;
    --cascade-from-operator) CASCADE_OP=$2; shift 2;;
    --since)                 CASCADE_SINCE=$2; shift 2;;
    --ssm-instance)          SSM_INSTANCE=$2; shift 2;;
    --ssm-region)            SSM_REGION=$2; shift 2;;
    -h|--help) sed -n '2,18p' "$0"; exit 0;;
    *) echo "unknown arg: $1" >&2; exit 2;;
  esac
done

[ -z "$SSM_INSTANCE" ] && { echo "--ssm-instance required" >&2; exit 2; }
: "${AWS_VAULT_PROFILE:?set AWS_VAULT_PROFILE to the aws-vault profile of the account that holds the instance}"
[ -z "$SSM_REGION" ]   && { echo "--ssm-region required" >&2; exit 2; }
[ -z "$KEY_NAME$CASCADE_OP" ] && { echo "--key-name or --cascade-from-operator required" >&2; exit 2; }

if [ -n "$CASCADE_OP" ]; then
  [ -z "$CASCADE_SINCE" ] && { echo "--cascade-from-operator requires --since RFC3339" >&2; exit 2; }
  SQL=$(cat <<SQL
WITH victims AS (
  SELECT sk.id, sk.name FROM setup_keys sk
  JOIN setup_keys_audit a ON a.key_id = sk.id AND a.event = 'create'
  WHERE a.created_by = '$CASCADE_OP' AND a.ts >= TIMESTAMPTZ '$CASCADE_SINCE'
    AND sk.revoked = false
)
UPDATE setup_keys
   SET revoked = true, updated_at = NOW()
 WHERE id IN (SELECT id FROM victims);
INSERT INTO setup_keys_audit (id, event, key_id, key_name, created_by, note)
  SELECT 'audit_rev_'||id||'_'||extract(epoch FROM now())::bigint,
         'revoke', id, name, '$(whoami)',
         'cascade from $CASCADE_OP since $CASCADE_SINCE (reason=$REASON)'
  FROM victims;
SELECT count(*) AS revoked_n FROM victims;
SQL
)
else
  SQL=$(cat <<SQL
UPDATE setup_keys
   SET revoked = true, updated_at = NOW()
 WHERE name = '$KEY_NAME' AND revoked = false;
INSERT INTO setup_keys_audit (id, event, key_id, key_name, created_by, note)
  SELECT 'audit_rev_'||id||'_'||extract(epoch FROM now())::bigint,
         'revoke', id, name, '$(whoami)', 'reason=$REASON'
  FROM setup_keys WHERE name = '$KEY_NAME';
SELECT name FROM setup_keys WHERE name = '$KEY_NAME';
SQL
)
fi

JSON=$(mktemp)
cat >"$JSON" <<JSON
{"commands":["docker exec -i postgres-a psql -U postgres netbird <<'EOF'\n$SQL\nEOF"]}
JSON

CMD_ID=$(aws-vault exec "$AWS_VAULT_PROFILE" --no-session -- aws ssm send-command \
  --region "$SSM_REGION" --instance-ids "$SSM_INSTANCE" \
  --document-name AWS-RunShellScript --parameters "file://$JSON" \
  --output text --query 'Command.CommandId')
for _ in $(seq 1 30); do
  STATUS=$(aws-vault exec "$AWS_VAULT_PROFILE" --no-session -- aws ssm get-command-invocation \
    --region "$SSM_REGION" --instance-id "$SSM_INSTANCE" --command-id "$CMD_ID" \
    --query 'Status' --output text 2>/dev/null || true)
  case "$STATUS" in
    Success|Failed|TimedOut|Cancelled) break;;
  esac
  sleep 2
done
OUT=$(aws-vault exec "$AWS_VAULT_PROFILE" --no-session -- aws ssm get-command-invocation \
  --region "$SSM_REGION" --instance-id "$SSM_INSTANCE" --command-id "$CMD_ID" \
  --query 'StandardOutputContent' --output text 2>/dev/null || true)
rm -f "$JSON"

echo "$OUT"
[ "$STATUS" = "Success" ]
