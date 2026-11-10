#!/usr/bin/env bash
# replace-single-use-setup-keys.sh — find people holding single-use mesh setup
# keys and issue reusable replacements.
#
# WHY THIS EXISTS
#   A `type: one-off, usage_limit: 1` setup key is a trap once handed out, and
#   fixing the minting path does not reach back to credentials already issued. A
#   person holding a spent key is blocked,
#   and a person holding an unfired one gets exactly one attempt — spend it on a
#   VM, a phone, or WSL and they are stuck.
#
# WHAT IT DOES
#   Lists every non-revoked one-off/limit-1 key on a goatnet, marks each as SPENT
#   or UNFIRED, and — only with --apply — mints a reusable replacement carrying
#   the SAME auto_groups. It never revokes, never deletes, and never touches the
#   original: a spent key stays spent, an unfired one keeps working.
#
# WHAT IT DELIBERATELY DOES NOT DO
#   Deliver anything. Minting a key does not tell the person it exists. Delivery
#   is a human step, and the supported path is the console's welcome resend so
#   the issue is recorded against the person rather than pasted into a chat. This
#   script prints the key values once, to the operator's terminal only.
#
# USAGE
#   ops/scripts/replace-single-use-setup-keys.sh <goatnet>            # dry run
#   ops/scripts/replace-single-use-setup-keys.sh <goatnet> --apply    # mint
#   ops/scripts/replace-single-use-setup-keys.sh <goatnet> --unfired-only  # at-risk only
#   ops/scripts/replace-single-use-setup-keys.sh <goatnet> --apply --only <name>
#
#   <goatnet> is the site id (for example site-a). The API token is
#   read from <SITE>_NB_API_TOKEN, matching the convention in
#   ops/ansible/operator-onboard.yml. Listing setup keys requires an ADMIN token;
#   NetBird has no read-only role that can enumerate them.
#
# SAFETY
#   Dry run is the default and prints exactly what --apply would mint. Machine
#   keys are EXCLUDED: a machine key is one-off by design (ADR 1096, one machine
#   per recorded custody edge) and making it reusable would be a real regression.
#   The filter is name-prefix based and imperfect, so review the plan before
#   applying.

set -euo pipefail

SITE="${1:-}"
shift || true

APPLY=0
ONLY=""
USAGE_LIMIT="${GOAT_REPLACEMENT_USAGE_LIMIT:-5}"
TTL_DAYS="${GOAT_REPLACEMENT_TTL_DAYS:-90}"

UNFIRED_ONLY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --apply) APPLY=1 ;;
    --only)  ONLY="${2:-}"; shift ;;
    --limit) USAGE_LIMIT="${2:-}"; shift ;;
    # Narrow to keys nobody has enrolled with yet. These are the people at
    # RISK rather than the people already blocked: they hold exactly one
    # attempt and have not taken it, so a wrong first move — a VM, a phone,
    # the WSL install instead of the native Windows client — strands them with
    # no self-service way back. A spent key is a different problem: that person
    # is already unable to add a device, but nothing they do today breaks, so
    # they can be handled when they surface.
    #
    # It also avoids churn. Re-minting for someone whose key is about to be
    # replaced by another process hands them two keys and no way to tell which
    # is current.
    --unfired-only) UNFIRED_ONLY=1 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

if [ -z "$SITE" ]; then
  echo "usage: $(basename "$0") <goatnet> [--apply] [--unfired-only] [--only <key-name>] [--limit N]" >&2
  exit 2
fi

# Site → management URL: https://<public_mgmt_fqdn>, the name the goatnet's own
# group_vars declare (ops/ansible/group_vars/<site>_hosts/vars.yml), the rule
# peer-attribution.py follows. NB_API names another.
GROUP_VARS="$(cd "$(dirname "${BASH_SOURCE[0]}")/../ansible/group_vars" && pwd)/$(echo "$SITE" | tr '-' '_')_hosts/vars.yml"
MGMT="${NB_API:-}"
if [ -z "$MGMT" ]; then
  FQDN="$(sed -nE "s/^public_mgmt_fqdn:[[:space:]]*[\"']?([A-Za-z0-9][A-Za-z0-9.-]*)[\"']?[[:space:]]*(#.*)?\$/\1/p" "$GROUP_VARS" 2>/dev/null | head -1 || true)"
  [ -n "$FQDN" ] || { echo "unknown goatnet '$SITE' — $GROUP_VARS declares no public_mgmt_fqdn; set NB_API=https://<management host>" >&2; exit 2; }
  MGMT="https://$FQDN"
fi

TOKEN_VAR="$(echo "$SITE" | tr 'a-z-' 'A-Z_')_NB_API_TOKEN"
TOKEN="${!TOKEN_VAR:-}"
if [ -z "$TOKEN" ]; then
  echo "no admin token in \$$TOKEN_VAR — listing setup keys needs the ADMIN role" >&2
  exit 2
fi

echo "goatnet   : $SITE ($MGMT)"
echo "mode      : $([ "$APPLY" = 1 ] && echo 'APPLY — will mint' || echo 'dry run')"
echo "replacement: type=reusable usage_limit=$USAGE_LIMIT ttl=${TTL_DAYS}d"
echo

KEYS="$(curl -fsS -m 30 -H "Authorization: Token $TOKEN" "$MGMT/api/setup-keys")"
PEERS="$(curl -fsS -m 30 -H "Authorization: Token $TOKEN" "$MGMT/api/peers")"

# Candidate = non-revoked, one-off, usage_limit 1, and not a machine/infra key.
# `used_times` separates SPENT (blocked now) from UNFIRED (one shot left).
PLAN="$(jq -r --arg only "$ONLY" --argjson unfired "$UNFIRED_ONLY" '
  [ .[]
    | select(.revoked == false)
    | select(.type == "one-off" and .usage_limit == 1)
    | select(.name | test("^peer-") )
    | select(.name | test("-(rpi|takserver|federation|probe|throwaway)") | not)
    | select($only == "" or .name == $only)
    | select($unfired == 0 or .used_times == 0)
  ] | sort_by(.name) | .[]
  | [.name, (.used_times|tostring), (.auto_groups|join(",")), .expires] | @tsv
' <<<"$KEYS")"

if [ -z "$PLAN" ]; then
  echo "no single-use person keys found — nothing to do."
  exit 0
fi

printf '%-46s %-9s %s\n' "KEY NAME" "STATE" "PEERS ENROLLED ON IT"
while IFS=$'\t' read -r name used groups _expires; do
  [ -z "$name" ] && continue
  if [ "$used" = "0" ]; then state="UNFIRED"; else state="SPENT"; fi
  enrolled="$(jq -r --arg n "$name" '
    [ .[] | select(.name | startswith($n)) | "\(.os)/\(if .connected then "up" else "down" end)" ]
    | if length == 0 then "(none)" else join(", ") end' <<<"$PEERS")"
  printf '%-46s %-9s %s\n' "${name:0:45}" "$state" "$enrolled"
done <<<"$PLAN"

COUNT="$(wc -l <<<"$PLAN" | tr -d ' ')"
echo
echo "$COUNT key(s) match."

if [ "$APPLY" != 1 ]; then
  echo "dry run — re-run with --apply to mint replacements."
  exit 0
fi

echo
echo "minting replacements — record these, they are shown once:"
echo

while IFS=$'\t' read -r name _used groups _expires; do
  [ -z "$name" ] && continue
  GROUPS_JSON="$(jq -cn --arg g "$groups" '$g | split(",") | map(select(length>0))')"
  BODY="$(jq -cn \
    --arg name "${name}-r" \
    --argjson groups "$GROUPS_JSON" \
    --argjson limit "$USAGE_LIMIT" \
    --argjson expires "$(( TTL_DAYS * 86400 ))" \
    '{name:$name, type:"reusable", expires_in:$expires,
      usage_limit:$limit, ephemeral:false, auto_groups:$groups,
      auto_peer_name:$name}')"
  RESP="$(curl -fsS -m 30 -X POST \
    -H "Authorization: Token $TOKEN" -H "Content-Type: application/json" \
    -d "$BODY" "$MGMT/api/setup-keys")" || { echo "FAILED: $name"; continue; }
  jq -r '"  \(.name)\n    key: \(.key)\n    devices: \(.usage_limit)   expires: \(.expires)"' <<<"$RESP"
done <<<"$PLAN"

cat <<'EOF'

Minting does not notify anyone. Deliver through the console's welcome resend so
the issue is recorded against the person — do not paste a key into chat, and do
not email a clickable credential link (institutional mail scanners fetch and
spend them; F-315).

The originals were left untouched: a spent key stays spent, an unfired one still
works. Revoke them only once the person confirms they are on the mesh.
EOF
