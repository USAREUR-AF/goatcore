#!/usr/bin/env bash
# kanidm-reissue-code.sh — issue a fresh credential-setup CODE for one person.
#
# This is the servicing end of the "reply and we'll issue another" promise the
# onboarding mail makes. When someone says their setup link expired or errored,
# run this and paste the two lines it prints into your reply. One command, so
# a stuck partner waits minutes rather than until somebody reconstructs the
# API dance.
#
# It prints a CODE and a BARE page address, never a clickable link carrying the
# token. There are now TWO independent reasons for that, and the second one was
# learned the hard way after the first:
#
#   1. F-315 — enterprise mail-security gateways fetch links in transit and
#      spend one-shot credential links, so the recipient is told the token was
#      "reused in another browser" and cannot proceed. Observed live:
#      5 of 98 issued links machine-fetched, every one an institutional domain,
#      two people stranded.
#
#   2. F-351 — a tokenised URL turns the reset page into a dead form. Kanidm's
#      /ui/reset submit button is htmx `hx-get="" hx-include="form"`, so it
#      re-issues the CURRENT url with the form field appended. Land on
#      /ui/reset?token=ABC and type anything into the box, and the browser asks
#      for /ui/reset?token=ABC&token=<typed> — two `token` params, which Kanidm
#      answers HTTP 400. htmx does not swap the DOM on a 4xx, so the page does
#      NOTHING, forever, with no message: the recipient sees a button that does
#      not work.
#
# A bare page address carries no state to spend AND leaves the form with one
# `token` param, so it submits correctly. Both problems have the same fix.
#
#   ops/scripts/kanidm-reissue-code.sh <handle>
#   ops/scripts/kanidm-reissue-code.sh someone --plane collab
#
# NOTE --ttl-hours above 24 does nothing: Kanidm caps credential-setup codes at
# 24 h (a 7-day request comes back as 24.00 h exactly, a 3-second one as the
# 5-minute floor). The script says so when you exceed it.
#
# Find who needs this before they email you:
#   ops/scripts/kanidm-door-audit.sh --reinvite-plan
#
# Credentials come from the environment (direnv), never arguments:
#   public plane (dashboard door) → GOAT_IDM_ADMIN_PW (or, when that is unset,
#     the variable the checkout names as public_door_idm_admin_pw_env)
#   collab plane (collab Kanidm) → COLLAB_KANIDM_IDM_ADMIN_PW
# The door's address is PUBLIC_DOOR_KANIDM_URL / COLLAB_KANIDM_URL, else what
# the checkout declares as public_door_kanidm_url / collab_door_kanidm_url in
# ops/ansible/group_vars/all.yml (https://idm.<zone>).
#
# Find the handle: it is the Kanidm account name, e.g. from the roster or
#   curl -H "Authorization: Bearer <tok>" https://<kanidm-host>/v1/person
# An account that has never completed setup carries neither `primary_credential`
# nor `passkeys` in its attrs — that is how you confirm someone is stuck rather
# than merely confused.
set -euo pipefail

HANDLE="${1:-}"; shift || true
PLANE=public
TTL_HOURS=24
while [ $# -gt 0 ]; do
  case "$1" in
    --plane)      PLANE="$2"; shift 2 ;;
    --ttl-hours)  TTL_HOURS="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$HANDLE" ]; then
  echo "usage: $0 <handle> [--plane public|collab] [--ttl-hours N]" >&2
  exit 2
fi

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
  *) echo "unknown --plane '$PLANE' (want: public | collab)" >&2; exit 2 ;;
esac
PW="${!PWVAR:-}"
HOST="${!HOSTVAR:-}"; [ -n "$HOST" ] || HOST="$(declared_var "$HOSTKEY")"; HOST="${HOST%/}"
[ -n "$HOST" ] || { echo "$HOSTVAR is not set and the checkout declares no $HOSTKEY — the base URL of the $PLANE door's Kanidm (https://idm.<zone>)." >&2; exit 2; }

if [ -z "$PW" ]; then
  echo "$PWVAR is not set — run under direnv in the repo, or export it." >&2
  exit 1
fi

hdrs=$(mktemp); trap 'rm -f "$hdrs"' EXIT

curl -sf -D "$hdrs" -o /dev/null --max-time 20 -X POST "$HOST/v1/auth" \
  -H 'Content-Type: application/json' -d '{"step":{"init":"idm_admin"}}'
SID=$(grep -i '^x-kanidm-auth-session-id:' "$hdrs" | tr -d '\r' | cut -d' ' -f2)
[ -n "$SID" ] || { echo "no auth session id from $HOST" >&2; exit 1; }

curl -sf -o /dev/null --max-time 20 -X POST "$HOST/v1/auth" \
  -H 'Content-Type: application/json' -H "x-kanidm-auth-session-id: $SID" \
  -d '{"step":{"begin":"password"}}'

# Every response is parsed defensively. Kanidm answers failures with an empty
# body or a bare string, and feeding that to a JSON parser buries the real
# problem under a Python traceback — which is precisely what you do not want
# when somebody is blocked and waiting on you.
jqfield() { python3 -c 'import sys,json
raw=sys.stdin.read().strip()
if not raw: sys.exit(0)
try: d=json.loads(raw)
except Exception: sys.exit(0)
if isinstance(d,dict):
    for k in sys.argv[1:]:
        d=d.get(k) if isinstance(d,dict) else None
    print(d or "")
else: print(d)' "$@"; }

TOKEN=$(curl -s --max-time 20 -X POST "$HOST/v1/auth" \
  -H 'Content-Type: application/json' -H "x-kanidm-auth-session-id: $SID" \
  --data-binary "{\"step\":{\"cred\":{\"password\":\"$PW\"}}}" | jqfield state success)
if [ -z "$TOKEN" ]; then
  echo "idm_admin auth failed on $HOST." >&2
  echo "  Most likely $PWVAR is stale — the password was rotated 2026-08-11." >&2
  exit 1
fi

RESP=$(curl -s -w '\n%{http_code}' --max-time 25 -H "Authorization: Bearer $TOKEN" \
  "$HOST/v1/person/$HANDLE/_credential/_update_intent/$((TTL_HOURS * 3600))")
STATUS=${RESP##*$'\n'}
CODE=$(printf '%s' "${RESP%$'\n'*}" | jqfield token)
# The response carries `expiry_time` (unix seconds) — the expiry the SERVER
# actually chose, which is not necessarily the one we asked for. Read it rather
# than computing now+TTL, because Kanidm 1.5.0 clamps HARD and silently:
#
#   requested 604800s (7 days) -> granted 86400s  (24 h exactly)
#   requested      3s          -> granted   300s  (5 min minimum)
#
# The ceiling is a server constant, not an
# account-policy value — `kanidm group account-policy` has auth-expiry and
# privilege-expiry and nothing for credential-update intents — so no goatnet can
# raise it by configuration, and asking for 48 h or 7 days silently yields 24 h.
# Telling a recipient "valid 7 days" on a 24 h credential is how somebody opens
# the mail on day three and finds a dead code and no explanation.
SERVER_EXPIRY=$(printf '%s' "${RESP%$'\n'*}" | jqfield expiry_time)

# Validate the SHAPE, never the status. Kanidm answers an unknown account with
# HTTP 200 and the bare JSON string "nomatchingentries" — so a status check
# passes and an admin cheerfully mails "nomatchingentries" to a partner as
# their setup code. A real token is four groups of five, dash-separated.
if ! printf '%s' "$CODE" | grep -Eq '^[A-Za-z0-9]{5}(-[A-Za-z0-9]{5}){3}$'; then
  case "$CODE:$STATUS" in
    nomatchingentries:*)
      echo "no account '$HANDLE' on $HOST — Kanidm returned 'nomatchingentries'." >&2
      echo "  The handle is the Kanidm account NAME, not the email address." >&2
      echo "  List them: curl -H \"Authorization: Bearer <tok>\" $HOST/v1/person" >&2 ;;
    *:500)
      echo "$HOST returned 500 on the write." >&2
      echo "  If the body says sqliteerror the database is wedged — see F-317." >&2
      echo "  A clean single-task restart is the fix, NOT a restore." >&2 ;;
    *)
      echo "did not get a usable code for '$HANDLE' (HTTP $STATUS)." >&2
      echo "  Server said: ${CODE:-<empty>}" >&2 ;;
  esac
  exit 1
fi

# Absolute expiry, not "valid 24h". The relative form is unreadable by the time
# it matters: a mail saying the link "expires 24 hours after it was issued" is
# something the reader cannot evaluate, and codes minted hours before the mail
# goes out have a shorter real budget than 24 h.
# A UTC deadline is a fact the reader can compare against their own clock.
# python3 rather than `date -d`: BSD date on the operator laptop has no -d.
EXPIRES=$(SERVER_EXPIRY="${SERVER_EXPIRY:-}" TTL_HOURS="$TTL_HOURS" python3 -c 'import os,time
se = os.environ.get("SERVER_EXPIRY","").strip()
t = int(se) if se.isdigit() else time.time() + int(os.environ["TTL_HOURS"]) * 3600
print(time.strftime("%Y-%m-%d %H:%M UTC", time.gmtime(t)))')

# Say so out loud when the server gave less than was asked for, rather than
# printing a shorter deadline and letting the operator assume their flag worked.
if [ -n "${SERVER_EXPIRY:-}" ] && [ "$TTL_HOURS" -gt 24 ] 2>/dev/null; then
  echo "note: asked for ${TTL_HOURS}h; Kanidm caps credential-setup codes at 24h and granted" >&2
  echo "      until $EXPIRES. The cap is a server constant, not a setting — there is no" >&2
  echo "      account-policy knob for it, so no goatnet can raise it. If a longer window is" >&2
  echo "      what you need, the answer is re-issuing on demand, not a bigger number here." >&2
fi

cat <<EOF

Paste this into your reply — as text, never as a link:

  1. Open this page:   $HOST/ui/reset
  2. In the box on that page, paste this code:
       $CODE
  3. Follow the page to set a password (or a passkey).

  Your username is "$HANDLE" — you need it AFTERWARDS, to log in at a
  dashboard. It is NOT what goes in the box on the reset page.

  The code stops working at $EXPIRES. If you get to it late, just
  reply and say so — a new one takes a minute to issue.

Two rules for whoever sends this:
  * Send the page address BARE, exactly as printed. Do not paste
    $HOST/ui/reset?token=$CODE
    — a mail scanner spends that link before the recipient opens it,
    and it also breaks the page's Submit button, silently.
  * Check afterwards that they actually got in, rather than assuming:
      ops/scripts/kanidm-door-audit.sh --plane $PLANE | grep -w $HANDLE
    No output means done. Still listed means still stuck: an account
    "unblocked by hand" can stay without a credential for days.
EOF
