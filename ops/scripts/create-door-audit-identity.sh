#!/usr/bin/env bash
# create-door-audit-identity.sh — mint the read-scoped Kanidm service account
# that door-credential-audit uses, on one door.
#
# OPERATOR-RUN, NEVER A PLAYBOOK SIDE EFFECT (live-state-guard-scheduling.md §4).
# Creating an identity-plane credential is a decision with an audit trail
# attached; it should happen because somebody ran a command that says so, not as
# a quiet consequence of converging a host. The role consumes what this prints
# and REFUSES to install without it.
#
# On privilege, stated plainly. The token this mints is READ-ONLY
# (`read_write: false`) and its bearer is a member of `idm_people_pii_read` and
# nothing else. That is genuine least privilege here, unlike the NetBird case:
# the audit needs to enumerate persons and see whether each holds a credential,
# and Kanidm has a read-only group that covers exactly that. The alternative the
# role falls back to — the `idm_admin` password — puts full identity-plane admin
# on a mgmt host to run two GETs.
#
# Usage:
#   ops/scripts/create-door-audit-identity.sh --plane public [--dry-run]
#   ops/scripts/create-door-audit-identity.sh --plane collab
#
# Needs, in the environment (direnv):
#   public → PUBLIC_DOOR_KANIDM_URL + GOAT_IDM_ADMIN_PW (or, when that is
#            unset, the variable the checkout names as
#            public_door_idm_admin_pw_env in ops/ansible/group_vars/all.yml)
#   collab → COLLAB_KANIDM_URL      + COLLAB_KANIDM_IDM_ADMIN_PW
# The URL is the base of that door's Kanidm (https://idm.<zone>). With the URL
# variable unset, the address is what the checkout declares as
# public_door_kanidm_url / collab_door_kanidm_url in
# ops/ansible/group_vars/all.yml.
#
# Idempotent: an existing service account is reused (its creation answers HTTP
# 409 with an attribute-uniqueness body, which is ACHIEVED, not failed — the
# same shape the collab provisioner learned the hard way in F-274). A SECOND
# api-token is minted each run, because Kanidm never returns an existing
# token's value; list and prune old ones with
#   kanidm service-account api-token status <account>
set -euo pipefail

PLANE=public
DRY_RUN=0
ACCOUNT=door-credential-audit

while [ $# -gt 0 ]; do
  case "$1" in
    --plane)   PLANE="$2"; shift 2 ;;
    --account) ACCOUNT="$2"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 64 ;;
  esac
done

# One value this checkout declares in ops/ansible/group_vars/all.yml (through
# project-setting.sh). Prints nothing where the script runs outside a checkout.
declared_var() {
  local helper
  helper="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/project-setting.sh"
  [ -x "$helper" ] || return 0
  "$helper" var "$1" 2>/dev/null || true
}

# The variable the public door's idm_admin password is read from:
# GOAT_IDM_ADMIN_PW. A checkout whose secret references carry another name
# declares it as public_door_idm_admin_pw_env, and that variable is read when
# GOAT_IDM_ADMIN_PW is unset.
public_pw_var() {
  local name=""
  [ -n "${GOAT_IDM_ADMIN_PW:-}" ] || name="$(declared_var public_door_idm_admin_pw_env)"
  case "$name" in ""|[0-9]*|*[!A-Za-z0-9_]*) name="" ;; esac
  if [ -n "$name" ] && [ -n "${!name:-}" ]; then printf '%s' "$name"; else printf 'GOAT_IDM_ADMIN_PW'; fi
}

case "$PLANE" in
  public) HOSTVAR=PUBLIC_DOOR_KANIDM_URL; HOSTKEY=public_door_kanidm_url; PWVAR="$(public_pw_var)" ;;
  collab) HOSTVAR=COLLAB_KANIDM_URL;      HOSTKEY=collab_door_kanidm_url; PWVAR=COLLAB_KANIDM_IDM_ADMIN_PW ;;
  *) echo "unknown --plane '$PLANE' (want: public | collab)" >&2; exit 64 ;;
esac
# The door's address is the goatnet's, not this script's: no built-in door.
PW="${!PWVAR:-}"
HOST="${!HOSTVAR:-}"; [ -n "$HOST" ] || HOST="$(declared_var "$HOSTKEY")"; HOST="${HOST%/}"
[ -n "$HOST" ] || { echo "$HOSTVAR is not set and the checkout declares no $HOSTKEY — the base URL of the $PLANE door's Kanidm (https://idm.<zone>)." >&2; exit 1; }
[ -n "$PW" ] || { echo "$PWVAR is not set — run under direnv in the repo." >&2; exit 1; }

if [ "$DRY_RUN" = 1 ]; then
  cat <<EOF
DRY RUN — would, on $HOST:
  1. create service account '$ACCOUNT' (display name "door credential audit")
  2. add it to group idm_people_pii_read   (read-only person + mail read)
  3. mint a READ-ONLY api-token and print it once
Nothing was changed.
EOF
  exit 0
fi

hdrs=$(mktemp); trap 'rm -f "$hdrs"' EXIT
curl -sf -D "$hdrs" -o /dev/null --max-time 20 -X POST "$HOST/v1/auth" \
  -H 'Content-Type: application/json' -d '{"step":{"init":"idm_admin"}}'
SID=$(grep -i '^x-kanidm-auth-session-id:' "$hdrs" | tr -d '\r' | cut -d' ' -f2)
[ -n "$SID" ] || { echo "no auth session id from $HOST" >&2; exit 1; }
curl -sf -o /dev/null --max-time 20 -X POST "$HOST/v1/auth" \
  -H 'Content-Type: application/json' -H "x-kanidm-auth-session-id: $SID" \
  -d '{"step":{"begin":"password"}}'
TOK=$(curl -s --max-time 20 -X POST "$HOST/v1/auth" \
  -H 'Content-Type: application/json' -H "x-kanidm-auth-session-id: $SID" \
  --data-binary "{\"step\":{\"cred\":{\"password\":\"$PW\"}}}" \
  | python3 -c 'import sys,json
try: print(json.load(sys.stdin).get("state",{}).get("success","") or "")
except Exception: print("")')
[ -n "$TOK" ] || { echo "idm_admin auth failed on $HOST — $PWVAR is probably stale." >&2; exit 1; }

api() {  # api <METHOD> <PATH> [<json-body>]
  if [ $# -ge 3 ]; then
    curl -s -w '\n%{http_code}' --max-time 25 -X "$1" "$HOST$2" \
      -H 'Content-Type: application/json' -H "Authorization: Bearer $TOK" -d "$3"
  else
    curl -s -w '\n%{http_code}' --max-time 25 -X "$1" "$HOST$2" \
      -H 'Content-Type: application/json' -H "Authorization: Bearer $TOK"
  fi
}

echo "1/3 service account '$ACCOUNT'"
r=$(api POST /v1/service_account \
     "{\"attrs\":{\"name\":[\"$ACCOUNT\"],\"displayname\":[\"door credential audit (read-only)\"]}}")
st=${r##*$'\n'}; body=${r%$'\n'*}
# "Already there" is ACHIEVED, not failed — and the shape of "already there" is
# NOT what the person endpoint uses:
#   POST /v1/person          repeat -> HTTP 409 {"conflicting_attributes":…}
#   POST /v1/service_account repeat -> HTTP 500 {"plugin":{"attrunique":…}}
# Keying idempotence on the status alone therefore turns a re-run of this script
# into a hard failure on a 500 that means success. This is the same mistake
# F-274 records against the collab provisioner, on a different endpoint.
case "$st:$body" in
  200:*|201:*)        echo "     created" ;;
  *attrunique*|*duplicate*|409:*)
                      echo "     already exists — reusing (ACHIEVED, not failed)" ;;
  *)                  echo "     FAILED: HTTP $st: $body" >&2; exit 1 ;;
esac

echo "2/3 group membership: idm_people_pii_read"
r=$(api POST "/v1/group/idm_people_pii_read/_attr/member" "[\"$ACCOUNT\"]")
st=${r##*$'\n'}; body=${r%$'\n'*}
case "$st" in
  200|201|204) echo "     added" ;;
  *)           echo "     FAILED: HTTP $st: $body" >&2; exit 1 ;;
esac

echo "3/3 read-only api-token"
#
# This step does NOT work over REST with an idm_admin password bearer, and the
# way it fails is the reason for all the shape-checking below:
#
#   POST /v1/service_account/<a>/_api_token   -> HTTP 403, body "accessdenied"
#   POST /v1/reauth  {"step":…}               -> HTTP 422 (wrong shape)
#   POST /v1/reauth  "token" | "cookie"       -> HTTP 500, body "invalidstate"
#
# Kanidm gates api-token minting behind a PRIVILEGED session, and a bearer
# obtained from the /v1/auth password flow is not one — reauth refuses it as
# invalidstate. The `kanidm` CLI performs that dance correctly, which is why it
# is the path here. (The same conclusion was already recorded, unproven, in
# collab-onboarding-provisioner.md §Follow-ons; this is the measurement.)
#
# The shape check is not defensive padding. Kanidm answers this endpoint with a
# BARE JSON STRING on refusal, so a naive `.token`-less parse hands the caller
# the word "accessdenied" and every downstream step then treats that as a
# credential — the same class of bug kanidm-reissue-code.sh already carries a
# guard for after Kanidm returned "nomatchingentries" as a setup code.
if command -v kanidm >/dev/null 2>&1; then
  echo "     using the kanidm CLI (privileged session)"
  echo "     run these two, then paste the printed token below:"
  echo "       kanidm login -D idm_admin -H $HOST"
  echo "       kanidm service-account api-token generate $ACCOUNT door-credential-audit --read-only -H $HOST"
else
  r=$(api POST "/v1/service_account/$ACCOUNT/_api_token" \
       '{"label":"door-credential-audit","expiry":null,"read_write":false}')
  st=${r##*$'\n'}; body=${r%$'\n'*}
  echo "     REST mint refused as expected: HTTP $st $(printf '%s' "$body" | head -c 40)" >&2
  cat >&2 <<EOF

  The service account and its group membership ARE created (steps 1 and 2), and
  re-running this is safe. What remains is the token, which needs the kanidm CLI:

      kanidm login -D idm_admin -H $HOST
      kanidm service-account api-token generate $ACCOUNT door-credential-audit --read-only

  The CLI is not in Homebrew; install with
      cargo install kanidm_tools
  or run it from a container image of the matching server version.

  UNTIL THEN the door-credential-audit role has one other way to run, and it is
  worse: door_credential_audit_admin_password puts the full idm_admin credential
  for this identity plane on a mgmt host. That is a deliberate trade, not an
  oversight — decide it, do not drift into it.
EOF
  exit 1
fi

cat <<EOF

Once you have the token, put it in .envrc.local.d (gitignored) and in the
goatnet group_vars as door_credential_audit_token, then deploy the role.

Kanidm returns a token value ONCE. If you lose it, mint another and prune the
old: kanidm service-account api-token status $ACCOUNT

Verify the scope is sufficient BEFORE trusting the guard — a token that can list
persons but cannot see credential attributes would report every person as
stranded, which is a worse lie than no guard at all:

  DOOR_AUDIT_TOKEN=<token> ops/scripts/kanidm-door-audit.sh --plane $PLANE

Compare its counts against the idm_admin run. They must match.
EOF
