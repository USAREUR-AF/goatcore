#!/usr/bin/env bash
# ory-group-mirror.sh — mirror-add (or remove) one Ory group on every Kratos
# identity that holds another. Runs ON the mgmt host against the site Kratos
# admin API (loopback :4434). Dry-run unless APPLY=1.
#
#   ory-group-mirror.sh add    OLD-NAME  NEW-NAME    # everyone with OLD-NAME also gets NEW-NAME (expand/migrate)
#   ory-group-mirror.sh remove NEW-NAME  OLD-NAME    # everyone with NEW-NAME loses OLD-NAME (contract)
#
# JSON-patch on metadata_public.groups (F-572 / ADR 1188 — login groups moved
# off self-editable traits), one identity at a time, verified by re-reading. A
# groups key in metadata_public, even null, marks the identity migrated and its
# traits are never read; only a not-yet-backfilled identity falls back to traits.
set -euo pipefail
MODE="${1:?add|remove}"; HAVE="${2:?group the identity must hold}"; TARGET="${3:?group to add/remove}"
KRATOS="${KRATOS_ADMIN:-http://127.0.0.1:4434}"
APPLY="${APPLY:-0}"
resp=$(mktemp "${TMPDIR:-/tmp}/ory-mirror-resp.XXXXXX")
trap 'rm -f "$resp"' EXIT
# Login groups come ONLY from admin-managed metadata_public (GHSA-9v3v):
# self-editable traits never count, even for a not-yet-backfilled identity.
LOGIN_GROUPS='def login_groups: ((.metadata_public // {}).groups // []);'

ids=$(curl -fsS -m 10 "$KRATOS/admin/identities?per_page=1000" \
  | jq -r --arg have "$HAVE" --arg tgt "$TARGET" --arg mode "$MODE" "$LOGIN_GROUPS"'
      .[] | login_groups as $g
          | select($g | index($have))
          | select( if $mode=="add" then ($g | index($tgt) | not)
                    else ($g | index($tgt)) end )
          | "\(.id) \(.traits.email) \($g | join(","))"')

if [[ -z "$ids" ]]; then echo "nothing to do: no identity holds $HAVE without/with $TARGET"; exit 0; fi
echo "plan ($MODE $TARGET on holders of $HAVE; APPLY=$APPLY):"
# Keep the established multiline plan formatting.
# shellcheck disable=SC2001
echo "$ids" | sed 's/^/  /'
[[ "$APPLY" == 1 ]] || { echo "dry-run; set APPLY=1 to execute"; exit 0; }

while read -r id email _groups; do
  # Kratos v1.3.1 creates a missing parent for an add on the child path. Writing
  # only groups preserves every sibling and avoids a client-side
  # read/merge/write; its admin endpoint does not support JSON-Patch test ops.
  patch=$(curl -fsS "$KRATOS/admin/identities/$id" | jq -c --arg g "$TARGET" --arg mode "$MODE" "$LOGIN_GROUPS"'
    login_groups as $cur
    | (if $mode == "remove" then $cur - [$g] elif ($cur | index($g)) then $cur else $cur + [$g] end) as $new
    | [{op:"add", path:"/metadata_public/groups", value:$new}]')
  code=$(curl -sS -o "$resp" -w '%{http_code}' -X PATCH \
         -H 'Content-Type: application/json' -d "$patch" "$KRATOS/admin/identities/$id")
  after=$(curl -fsS "$KRATOS/admin/identities/$id" | jq -r "$LOGIN_GROUPS"' login_groups | join(",")')
  echo "  $id $email -> HTTP $code; groups now: $after"
  [[ "$code" == 200 ]] || { echo "  FAILED: $(cat "$resp")"; exit 1; }
done <<< "$ids"
