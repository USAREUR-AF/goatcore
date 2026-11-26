#!/usr/bin/env bash
# identity-roundtrip.sh — the write leg of tutorial 2's checkpoint 3: a row
# written at the founder appears at the joiner, and its deletion at the joiner
# comes back to the founder (docs/tutorials/02-first-federated-node.md).
#
#   ops/validation/identity-roundtrip.sh --write --founder <url> --joiner <url> [--timeout <s>]
#
# THIS WRITES. It creates one identity in the founder's Kratos and deletes it at
# the joiner, through each site's Kratos admin API (<url> is the admin API's
# base, such as an ssh -L forward to the host's admin port). It never runs
# inside `goatbuilder assay`, which only reads, and it does nothing without
# --write. On a live goatnet it is optional.
#
# The identity is obviously a test: goat-roundtrip-<UTC stamp>-<random>@example.org,
# created inactive (it cannot sign in) with a random password nobody sees, so
# its address is a credential identifier both sites can look up. The script
# refuses, writing nothing, when that address already exists at either site.
# On any failure after the create it still deletes the identity at both sites
# and says what it deleted; when it cannot, it names the identity and the
# command that removes it.
#
# Exit 0  the round trip passed and the identity is gone at both sites
#      1  the round trip failed; the identity was deleted at both sites
#      2  refused or a usage error; nothing was written
#      3  the identity could not be removed; the message names it
#
# Needs curl and python3. Bash 3.2 portable.
set -euo pipefail

usage() { echo "usage: identity-roundtrip.sh --write --founder <kratos admin url> --joiner <kratos admin url> [--timeout <seconds>]" >&2; exit 2; }

WRITE=0; FOUNDER=""; JOINER=""; TIMEOUT=90; EMAIL=""
while [ $# -gt 0 ]; do
  case "$1" in
    --write) WRITE=1 ;;
    --founder) [ $# -ge 2 ] || usage; FOUNDER="${2%/}"; shift ;;
    --joiner) [ $# -ge 2 ] || usage; JOINER="${2%/}"; shift ;;
    --timeout) [ $# -ge 2 ] || usage; TIMEOUT="$2"; shift ;;
    # For the partition harness: a fixed address, still a test one.
    --email) [ $# -ge 2 ] || usage; EMAIL="$2"; shift ;;
    -h|--help) sed -n '2,26p' "$0"; exit 0 ;;
    *) usage ;;
  esac
  shift
done
[ -n "$FOUNDER" ] && [ -n "$JOINER" ] || usage
case "$TIMEOUT" in ''|*[!0-9]*) usage ;; esac
if [ "$WRITE" != 1 ]; then
  echo "identity-roundtrip: refused: this WRITES to both sites' identity stores (it creates one test identity at the founder and deletes it at the joiner). Pass --write to run it; nothing was written." >&2
  exit 2
fi
if [ -z "$EMAIL" ]; then
  EMAIL="goat-roundtrip-$(date -u +%Y%m%dT%H%M%SZ)-$(python3 -I -c 'import secrets; print(secrets.token_hex(4))')@example.org"
fi
case "$EMAIL" in
  goat-roundtrip-*@example.org) ;;
  *) echo "identity-roundtrip: refused: --email must be goat-roundtrip-<anything>@example.org; nothing was written" >&2; exit 2 ;;
esac

WORK="$(mktemp -d)"
ID=""; ATTEMPTED=0; CREATED=0; DONE=0; CLEANED=0; POLLER=""
# Every call is bounded, so a stalled admin API (a dead ssh -L forward) ends the run.
CURL=(curl -sS -m 10 --connect-timeout 5)
ENC="$(python3 -I -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$EMAIL")"
# http <METHOD> <url> [json body]  -> prints the status code; the body is in $WORK/body
http() {
  local c
  : > "$WORK/body"
  if [ $# -ge 3 ]; then
    c="$("${CURL[@]}" -o "$WORK/body" -w '%{http_code}' -X "$1" "$2" -H 'Content-Type: application/json' --data-binary "$3" 2>"$WORK/err")" || c=000
  else
    c="$("${CURL[@]}" -o "$WORK/body" -w '%{http_code}' -X "$1" "$2" 2>"$WORK/err")" || c=000
  fi
  echo "${c:-000}"
}
side() { if [ "$1" = "$FOUNDER" ]; then echo founder; else echo joiner; fi; }
exists() { # exists <url> -> 200 present, 404 absent, other: the code
  http GET "$1/admin/identities/$ID"
}
# lookup <url> -> prints the code of a read by address; the hits are in $WORK/body
lookup() { http GET "$1/admin/identities?credentials_identifier=$ENC"; }
hits() { python3 -I -c 'import json,sys; print(len(json.load(open(sys.argv[1]))))' "$WORK/body" 2>/dev/null || echo unreadable; }
hit_id() { python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))[0]["id"])' "$WORK/body"; }

cleanup() {
  local rc=$? left="" u code
  [ "$CLEANED" = 1 ] && return
  CLEANED=1
  trap '' INT TERM HUP
  if [ -n "$POLLER" ]; then kill "$POLLER" 2>/dev/null || true; wait "$POLLER" 2>/dev/null || true; fi
  # A create whose reply was lost, or cut short by a signal, may still have
  # committed: the address finds it.
  if [ "$ATTEMPTED" = 1 ] && [ "$CREATED" != 1 ]; then
    code="$(lookup "$FOUNDER")"
    if [ "$code" = 200 ] && [ "$(hits)" = 1 ]; then
      ID="$(hit_id)"; CREATED=1
      echo "identity-roundtrip: the founder holds $ID ($EMAIL) from this run's create; deleting it"
    elif [ "$code" = 200 ] && [ "$(hits)" = 0 ]; then
      echo "identity-roundtrip: the founder holds no $EMAIL; nothing was written" >&2
    else
      echo "identity-roundtrip: CLEANUP FAILED: could not tell whether the founder created $EMAIL (a read answered $code). Look it up: curl '<founder admin url>/admin/identities?credentials_identifier=$ENC'"
      rc=3
    fi
  fi
  if [ "$CREATED" = 1 ] && [ "$DONE" != 1 ]; then
    for u in "$FOUNDER" "$JOINER"; do
      code="$(http DELETE "$u/admin/identities/$ID")"
      case "$code" in
        204) echo "identity-roundtrip: cleaned up: deleted $ID at the $(side "$u")" ;;
        404) ;;
        *) echo "identity-roundtrip: cleanup: DELETE at the $(side "$u") answered $code" ;;
      esac
    done
    for u in "$FOUNDER" "$JOINER"; do
      code="$(exists "$u")"
      [ "$code" = 404 ] || left="$left $(side "$u") ($code)"
    done
    if [ -n "$left" ]; then
      echo "identity-roundtrip: CLEANUP FAILED: identity $ID ($EMAIL) may remain at:$left. Remove it with: curl -X DELETE <that site's admin url>/admin/identities/$ID"
      rc=3
    elif [ "$rc" = 0 ] || [ "$rc" = 2 ]; then
      rc=1
    fi
  fi
  rm -rf "$WORK"
  exit "$rc"
}
trap cleanup EXIT
# A signal ends the run with the shell's code for it; cleanup runs once, on EXIT.
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

# Refuse, writing nothing, when the address is already there, or a site does not answer.
for u in "$FOUNDER" "$JOINER"; do
  code="$(lookup "$u")"
  if [ "$code" != 200 ]; then
    echo "identity-roundtrip: refused: the $(side "$u")'s admin API ($u) answered $code to a read: $(head -c 200 "$WORK/err" "$WORK/body" 2>/dev/null | tr '\n' ' '); nothing was written" >&2
    exit 2
  fi
  if [ "$(hits)" != 0 ]; then
    echo "identity-roundtrip: refused: $EMAIL already exists at the $(side "$u"); nothing was written" >&2
    exit 2
  fi
done

BODY="$(python3 -I -c '
import json, secrets, sys
print(json.dumps({"schema_id": "default", "state": "inactive",
  "traits": {"email": sys.argv[1], "name": "goat round-trip probe (deleted by identity-roundtrip.sh)"},
  "credentials": {"password": {"config": {"password": secrets.token_urlsafe(32)}}}}))' "$EMAIL")"
ATTEMPTED=1
code="$(http POST "$FOUNDER/admin/identities" "$BODY")"
if [ "$code" != 201 ] || ! ID="$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$WORK/body" 2>/dev/null)"; then
  echo "identity-roundtrip: the founder answered HTTP $code to the create: $(head -c 300 "$WORK/body" | tr '\n' ' '); checking whether it created the identity anyway" >&2
  ID=""
  exit 2
fi
CREATED=1
echo "identity-roundtrip: created $ID ($EMAIL) at the founder, inactive"

# wait_for <url> <code> -> seconds taken, or fails at the timeout. It runs as a
# background job (poll), so a signal ends the wait at once; cleanup stops it.
wait_for() {
  local start now code
  start="$(date +%s)"
  while :; do
    code="$(exists "$1")"
    now="$(date +%s)"
    [ "$code" = "$2" ] && { echo $((now - start)); return 0; }
    [ $((now - start)) -ge "$TIMEOUT" ] && return 1
    sleep 1
  done
}

poll() { # poll <url> <code> -> 0 and the seconds in $WORK/poll, or 1 at the timeout
  wait_for "$1" "$2" > "$WORK/poll" &
  POLLER=$!
  local rc=0
  wait "$POLLER" || rc=$?
  POLLER=""
  return "$rc"
}

if poll "$JOINER" 200; then seen="$(cat "$WORK/poll")"; else
  echo "identity-roundtrip: FAIL: $ID did not reach the joiner within ${TIMEOUT}s: the founder's write is not replicating to the joiner"
  exit 1
fi
echo "identity-roundtrip: $ID is at the joiner after ${seen}s"
code="$(http DELETE "$JOINER/admin/identities/$ID")"
if [ "$code" != 204 ]; then
  echo "identity-roundtrip: FAIL: the joiner did not delete $ID (HTTP $code)"
  exit 1
fi
echo "identity-roundtrip: deleted $ID at the joiner"
if poll "$FOUNDER" 404; then gone="$(cat "$WORK/poll")"; else
  echo "identity-roundtrip: FAIL: the joiner's deletion of $ID did not reach the founder within ${TIMEOUT}s: the joiner's writes are not replicating to the founder"
  exit 1
fi
code="$(exists "$JOINER")"
if [ "$code" != 404 ]; then
  echo "identity-roundtrip: FAIL: $ID is gone at the founder but the joiner answers $code"
  exit 1
fi
DONE=1
echo "identity-roundtrip: PASS $ID: created at the founder, at the joiner after ${seen}s, deleted at the joiner, gone at the founder after ${gone}s"
