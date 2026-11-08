#!/usr/bin/env bash
# onboard-intake-smoke.sh — prove the candidate intake path on a LIVE goatnet,
# the way a candidate experiences it: mint a one-time invite, open the public
# intake page in a real Chromium, fill the form, click "Submit for review",
# require the real 201, then reject the proposal it created so nothing is left
# in anyone's queue. Exit 0 = the path a candidate walks works on that goatnet.
#
# This is the release acceptance gate for the intake form on the dev goatnet
# (docs/design/three-stage-candidate-onboarding.md §"Acceptance gate"): a broken
# page is invisible to every server-side test and to every log line, because
# the page's script throws before its fetch.
# Only driving the deployed page catches that class.
#
# Usage:
#   ops/scripts/onboard-intake-smoke.sh --context <goat-context> [--env <goatnet>] [--keep]
#
#   --context   goat CLI context whose key may mint invites and reject its own
#               invitees (a sponsor is enough; no operator key needed).
#   --env       goatnet name for `goat admin invite --env` (default: the context).
#   --stage     intake (default): mint with the tier pre-bound, submit, reject.
#               sponsor: mint with EVERY field left open (the shape the web
#               invite page produces), submit with the candidate choosing, then
#               sponsor-approve — the stage-2b gate must ACCEPT it — then
#               withdraw that vouch (sponsor-reject, ADR 1207). This is the row
#               that matters: an open-field invite must not land as an incomplete
#               draft both gates refuse.
#   --keep      leave the proposal in place instead of rejecting it.
#   --goat-via  <user@host>: run the `goat` legs on that host over ssh (it must
#               hold the context), keeping the browser leg local. For a goatnet
#               this laptop is not meshed to, reached through its door
#               (docs/design/env-doors.md): the intake page is public, the
#               console is mesh-only, and the context stays on the host.
#
# Needs: goat (PATH), node + npm, and a one-time
#   (cd ops/goat-id-console/web/e2e && npm ci && npx playwright install chromium)
set -euo pipefail

CTX="" ; ENV_NAME="" ; KEEP=0 ; VIA="" ; STAGE="intake"
while [ $# -gt 0 ]; do
  case "$1" in
    --context)  CTX="$2"; shift 2 ;;
    --env)      ENV_NAME="$2"; shift 2 ;;
    --keep)     KEEP=1; shift ;;
    --goat-via)
      # An empty value (the acceptance manifest renders `--goat-via {{ goat_via }}`
      # and the local case leaves the var blank) means "run goat locally".
      if [ $# -ge 2 ] && [ -n "$2" ] && [ "${2#--}" = "$2" ]; then VIA="$2"; shift 2; else VIA=""; shift; fi ;;
    --stage)    STAGE="$2"; shift 2 ;;
    -h|--help) sed -n '2,35p' "$0"; exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done
[ -n "$CTX" ] || { echo "usage: $0 --context <goat-context> [--env <goatnet>] [--stage intake|sponsor] [--goat-via user@host] [--keep]" >&2; exit 2; }
case "$STAGE" in intake|sponsor) ;; *) echo "smoke: --stage must be intake or sponsor" >&2; exit 2 ;; esac
[ -n "$ENV_NAME" ] || ENV_NAME="$CTX"

# via_ssh: run a command on the --goat-via host. VIA is user@host, or
# user@host:port for a door address used directly. When the release driver
# reaches the host through a door it names the door in GOAT_ROWS_SSH_JUMP
# (user@127.0.0.1:port, env-door.sh <env> --ssh-target) and VIA is the host's
# mesh address (env-door.sh <env> --host-target): the ssh jumps through the door,
# as ops/validation/probes.py _ssh_jump does (docs/design/env-doors.md).
via_ssh() {
  local _host="$VIA" _port="" _jump=()
  case "$VIA" in *:*) _host="${VIA%:*}"; _port="${VIA##*:}" ;; esac
  if [ -n "${GOAT_ROWS_SSH_JUMP:-}" ]; then
    local _jt="${GOAT_ROWS_SSH_JUMP%:*}" _jp="${GOAT_ROWS_SSH_JUMP##*:}"
    _jump=(-o "ProxyCommand=ssh -o BatchMode=yes -o ConnectTimeout=6 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -p $_jp -W %h:%p $_jt")
  fi
  ssh -o BatchMode=yes -o ConnectTimeout=10 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
    ${_jump[@]+"${_jump[@]}"} ${_port:+-p "$_port"} "$_host" "$@"
}

# goat: local, or on a host that holds the context (see --goat-via).
goat() {
  if [ -n "$VIA" ]; then via_ssh goat "$@"; else command goat "$@"; fi
}

# on_goat_host: a shell line where goat runs (local, or the --goat-via host),
# for the one read the CLI has no verb for (the console's capabilities).
on_goat_host() {
  if [ -n "$VIA" ]; then via_ssh "$1"; else sh -c "$1"; fi
}

# q: an argument for goat — shell-escaped only when it rides over ssh, where the
# remote shell re-parses the command line.
q() { if [ -n "$VIA" ]; then printf %q "$1"; else printf %s "$1"; fi; }

# The goat legs need their contexts on whichever machine runs goat. A context
# that is not there makes `goat admin` answer E_NOT_ADMIN with exit 4, the same
# code as a real refusal, so without this check a runner that was never given
# its contexts reads as a broken candidate. Missing setup is exit 2: the row
# could not run here, which says nothing about the goatnet.
need_context() {
  local rc=0
  # GNU Bash 3.2 can exit inside the command wrapper before this handler
  # sees a failed probe. Disable errexit only in the read-only probe child;
  # the parent keeps strict failure handling for every mutating goat call.
  if (set +e; goat --context "$1" profile show) >/dev/null 2>&1; then
    return 0
  else
    rc=$?
  fi
  case "$rc" in
    127) echo "smoke: goat is not installed on ${VIA:-this machine}" >&2 ;;
    255) echo "smoke: could not reach $VIA over ssh" >&2 ;;
    *)   echo "smoke: goat context '$1' is not initialized on ${VIA:-this machine} (goat --context $1 profile init <bundle>)" >&2 ;;
  esac
  return 2
}
need_context "$CTX" || exit "$?"

HERE="$(cd "$(dirname "$0")" && pwd)"
E2E="$HERE/../goat-id-console/web/e2e"
[ -d "$E2E/node_modules" ] || { echo "smoke: $E2E has no node_modules — run: (cd $E2E && npm ci && npx playwright install chromium)" >&2; exit 2; }

TS="$(date -u +%Y%m%dT%H%M%SZ)"
NAME="Intake Smoke $TS"
# Reserved TLD (RFC 2606): nothing can deliver to it, so the sponsor/rejection
# mails the flow sends about this candidate reach no real inbox.
EMAIL="intake-smoke-$TS@example.invalid"
NOTE="intake smoke $TS — synthetic, rejected by ops/scripts/onboard-intake-smoke.sh"

if [ "$STAGE" = sponsor ] && [ "$KEEP" = 0 ]; then
  # The sponsor stage vouches, then withdraws the vouch (ADR 1207). Check the
  # console can take the withdrawal BEFORE vouching: a console without it would
  # leave a synthetic candidate sponsor-approved, with no operator key here to
  # clear it. The console advertises it at /api/onboard/capabilities.
  CAPS="$(on_goat_host "ep=\$(sed -n 's/^onboard_endpoint *= *\"\(.*\)\"/\\1/p' ~/.goat/contexts/$CTX/profile.toml | head -1); [ -n \"\$ep\" ] && curl -s --max-time 10 \"\$ep/api/onboard/capabilities\"" 2>/dev/null || true)"
  case "$CAPS" in
    *'"sponsor_withdraw":true'*) ;;
    *) echo "smoke: UNTESTED, nothing created — the console on $ENV_NAME predates ADR 1207 sponsor withdrawal" >&2
       exit 2 ;;
  esac
fi

echo "smoke: minting a one-time invite on $ENV_NAME via context $CTX${VIA:+ (goat on $VIA)}"
if [ "$STAGE" = sponsor ]; then
  # Every field open: tier, affiliation, site, quota — what the web invite page mints.
  INV_JSON="$(goat admin invite --context "$CTX" --env "$ENV_NAME" --ttl-hours 1 --note "$(q "$NOTE")" --json)"
else
  INV_JSON="$(goat admin invite --context "$CTX" --env "$ENV_NAME" --tier browse --ttl-hours 1 --note "$(q "$NOTE")" --json)"
fi
INV_JSON="$(printf '%s' "$INV_JSON" | grep -v "TLS server")"
INTAKE_URL="$(printf '%s' "$INV_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["intake_url"])')"
INV_ID="$(printf '%s' "$INV_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')"
echo "smoke: invite $INV_ID → ${INTAKE_URL%%\?*}?t=oti_…"

cleanup_invite() {
  # The browser never submitted: the invite is still open — revoke it.
  echo "smoke: submit did not land; revoking the open invite $INV_ID"
  goat admin invites revoke "$INV_ID" --context "$CTX" >/dev/null 2>&1 || true
}

echo "smoke: driving the public intake page in Chromium"
# Only the intake spec has a live test. The other specs drive local fixtures
# and stubs that live mode does not start, so they cannot pass here.
if ! (cd "$E2E" && INTAKE_URL="$INTAKE_URL" INTAKE_NAME="$NAME" INTAKE_EMAIL="$EMAIL" npx playwright test tests/intake-form.spec.ts --reporter=list); then
  cleanup_invite
  echo "smoke: FAIL — the candidate intake path on $ENV_NAME does not accept a submit"
  exit 1
fi

echo "smoke: locating the proposal the submit created"
PROP_ID="$(goat admin queue ls --status proposed --context "$CTX" --env "$ENV_NAME" --json | grep -v "TLS server" \
  | python3 -c 'import json,sys; e=sys.argv[1]; print(next((p["id"] for p in json.load(sys.stdin) if p.get("email")==e), ""))' "$EMAIL")"
if [ -z "$PROP_ID" ]; then
  echo "smoke: FAIL — the browser got a 201 but no proposal for $EMAIL is in the queue on $ENV_NAME" >&2
  exit 1
fi
echo "smoke: proposal $PROP_ID is in 'proposed' (stage 2 reached)"

if [ "$STAGE" = sponsor ]; then
  echo "smoke: sponsor-approving $PROP_ID (stage 2b) — the gate must accept an open-field invite"
  if ! goat admin queue sponsor-approve "$PROP_ID" --context "$CTX" 2>&1 | grep -v "TLS server"; then
    echo "smoke: FAIL — the sponsor gate refused $PROP_ID on $ENV_NAME (an open-field invite is stranded at review)"
    [ "$KEEP" = 1 ] || goat admin queue sponsor-reject "$PROP_ID" --context "$CTX" --note "$(q "$NOTE")" >/dev/null 2>&1 || true
    exit 1
  fi
  if [ "$KEEP" = 1 ]; then
    echo "smoke: --keep set; leaving $PROP_ID sponsor-approved"
  else
    # Withdraw the vouch as the same sponsor (ADR 1207): no operator key needed.
    if goat admin queue sponsor-reject "$PROP_ID" --context "$CTX" --note "$(q "$NOTE")" >/dev/null 2>&1; then
      echo "smoke: $PROP_ID vouch withdrawn by the sponsor; nothing left in the queue"
    else
      echo "smoke: FAIL — the gate accepted $PROP_ID, but the sponsor could not withdraw it; the synthetic proposal is left sponsor-approved on $ENV_NAME for an admin to reject" >&2
      exit 1
    fi
  fi
  echo "smoke: PASS — open-field invite on $ENV_NAME: submit lands (201), proposal complete, sponsor gate accepts"
  exit 0
fi
if [ "$KEEP" = 1 ]; then
  echo "smoke: --keep set; leaving $PROP_ID in the queue"
else
  goat admin queue sponsor-reject "$PROP_ID" --context "$CTX" --note "$(q "$NOTE")" >/dev/null
  echo "smoke: $PROP_ID rejected; nothing left in the queue"
fi
echo "smoke: PASS — candidate intake on $ENV_NAME: page loads, script runs, submit lands (201), proposal recorded"
