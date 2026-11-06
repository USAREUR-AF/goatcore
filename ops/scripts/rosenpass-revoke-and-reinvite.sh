#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# rosenpass-revoke-and-reinvite.sh — run the Rosenpass revocation from an
# operator laptop, against a goatnet's mgmt host.
#
# DRY RUN BY DEFAULT. --apply is required to change anything.
#
# This is a THIN DRIVER. All of the logic lives in the sibling
# rosenpass_revoke_and_reinvite.py, which runs ON the mgmt host because that is
# where the credentials are: the NetBird API token and the console's notify
# token are both container environment, and copying either to a laptop to make
# a script simpler is not a trade worth taking.
#
# WHY THIS FILE STOPPED CARRYING ITS OWN PRECONDITION. It used to. Two versions
# of it, in fact, and both were wrong in ways that looked like diligence:
#
#   1. The first POSTed to the console's notify API with a `dry_run` field and
#      grepped the response for the flag. The handler has no dry_run support, so
#      the body could never contain the command, AND the console hostname does
#      not resolve from a laptop — so it was failing on DNS, not on content. It
#      refused every time. A guard that cannot pass is worse than no guard: it
#      blocks the correct action while looking careful.
#   2. The replacement grepped the deployed BINARY for the literal string
#      "--enable-rosenpass=false". That is falsifiable and it passes, but it
#      proves the wrong thing — that the code CAN emit the flag, not that the
#      deployed config WILL. The command is rendered from NOTIFY_ROSENPASS. Had
#      that been true, the reissue mail would have told every affected owner to
#      turn Rosenpass back ON, which is the exact inverse of the point.
#
# The Python checks the VALUE of NOTIFY_ROSENPASS on the running console. One
# precondition, in the place that can actually evaluate it. Two preconditions
# that can disagree is its own failure mode.
#
# usage:
#   ops/scripts/rosenpass-revoke-and-reinvite.sh <goatnet>                    # dry run
#   ops/scripts/rosenpass-revoke-and-reinvite.sh <goatnet> --apply
#   ops/scripts/rosenpass-revoke-and-reinvite.sh <goatnet> --apply --only-offline
#   ops/scripts/rosenpass-revoke-and-reinvite.sh <goatnet> --apply --limit=3
#
# <goatnet> names the goatnet (site-a or site_a). The driver reads the rest from
# that goatnet's configuration (ops/scripts/goatnet-facts.py) and hands it to the
# Python as GOATNET and NB_API:
#   the management host   its first management host in
#                         ops/ansible/inventory/doors.yaml
#   the management API    https://<public_mgmt_fqdn> from its group_vars
#
# env:
#   MGMT_HOST   user@host of the management host to drive (default: from doors.yaml)
#   NB_API      management API URL (default: from the goatnet's group_vars)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PY="${HERE}/rosenpass_revoke_and_reinvite.py"
GOATNET="${1:-}"
case "$GOATNET" in ""|-*) echo "usage: $(basename "$0") <goatnet> [--apply] [--only-offline] [--limit=N]" >&2; exit 2 ;; esac
shift
case "$GOATNET" in *[!A-Za-z0-9_-]*) echo "rosenpass-revoke-and-reinvite: '$GOATNET' is not a goatnet name" >&2; exit 2 ;; esac
for a in "$@"; do
  case "$a" in --apply|--only-offline|--limit=[0-9]*) ;; *) echo "rosenpass-revoke-and-reinvite: unknown argument $a" >&2; exit 2 ;; esac
done
MGMT_HOST="${MGMT_HOST:-$(python3 "$HERE/goatnet-facts.py" mgmt-ssh "$GOATNET")}" \
  || { echo "rosenpass-revoke-and-reinvite: no management host for $GOATNET; set MGMT_HOST=user@host" >&2; exit 2; }
NB_API="${NB_API:-$(python3 "$HERE/goatnet-facts.py" mgmt-url "$GOATNET")}" \
  || { echo "rosenpass-revoke-and-reinvite: no management API for $GOATNET; set NB_API=https://<management host>" >&2; exit 2; }
case "$NB_API" in https://*) ;; *) echo "rosenpass-revoke-and-reinvite: NB_API must be an https:// URL" >&2; exit 2 ;; esac
case "$NB_API" in *[!A-Za-z0-9.:/_-]*) echo "rosenpass-revoke-and-reinvite: NB_API carries characters a URL here does not" >&2; exit 2 ;; esac

[ -f "$PY" ] || { echo "missing $PY" >&2; exit 2; }

# WHAT YOU SHIP IS YOUR WORKING COPY, NOT main. The line below pipes the local
# file over SSH and OVERWRITES the copy on the mgmt host, so running this from a
# stale checkout silently reinstates whatever that checkout contains and undoes
# fixes that are already merged. This is not hypothetical: the
# mint in the Python was changed from a single-use key to a reusable one
# (dbf09c3e3) hours before a scheduled run over 38 owners — shipping yesterday's
# copy would have handed every one of them a key good for exactly one device,
# and nothing downstream would have said so.
#
# Compared against origin/main rather than "is my branch behind", because the
# only thing that matters is the bytes about to be written to the host. Divergence
# is not automatically wrong — you may be testing a change — so this refuses only
# --apply, and names the difference either way.
if command -v git >/dev/null && git -C "$HERE" rev-parse --git-dir >/dev/null 2>&1; then
  # Via a temp file, not "$(git show)": command substitution strips trailing
  # newlines, so a byte-identical file compares as different and the guard cries
  # wolf on every clean checkout. A guard that always fires gets bypassed by
  # reflex, which is worse than not having one.
  _ref_py="$(mktemp)"
  trap 'rm -f "$_ref_py"' EXIT
  git -C "$HERE" fetch -q origin main 2>/dev/null || true
  git -C "$HERE" show origin/main:ops/scripts/rosenpass_revoke_and_reinvite.py > "$_ref_py" 2>/dev/null || : > "$_ref_py"
  if [ -s "$_ref_py" ] && ! diff -q "$_ref_py" "$PY" >/dev/null 2>&1; then
    echo "!! the script you are about to ship DIFFERS from origin/main:" >&2
    # `|| true` on both halves: diff exits 1 when files differ, which is the
    # whole reason we are here, and `set -euo pipefail` would otherwise abort
    # the script at the moment it has something to say.
    { diff -u "$_ref_py" "$PY" || true; } | { sed -n '1,40p' || true; } >&2
    case " $* " in
      *" --apply "*)
        if [ "${RP_ALLOW_LOCAL_SCRIPT:-0}" != "1" ]; then
          echo "" >&2
          echo "refusing --apply from a checkout that does not match origin/main." >&2
          echo "  git pull, or re-run with RP_ALLOW_LOCAL_SCRIPT=1 if the difference is deliberate." >&2
          exit 3
        fi
        echo "RP_ALLOW_LOCAL_SCRIPT=1 — shipping the local version deliberately." >&2
        ;;
      *) echo "(dry run — continuing; --apply would refuse without RP_ALLOW_LOCAL_SCRIPT=1)" >&2 ;;
    esac
  fi
fi

echo "==> driving ${MGMT_HOST} (goatnet ${GOATNET}, ${NB_API})"
ssh -o BatchMode=yes "${MGMT_HOST}" \
  'mkdir -p ~/rp-notice && cat > ~/rp-notice/rosenpass_revoke_and_reinvite.py' < "$PY"
exec ssh -o BatchMode=yes "${MGMT_HOST}" \
  "cd ~/rp-notice && GOATNET=${GOATNET} NB_API=${NB_API} python3 rosenpass_revoke_and_reinvite.py $*"
